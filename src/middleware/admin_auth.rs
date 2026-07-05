use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use jsonwebtoken::{crypto, get_current_timestamp, Algorithm, DecodingKey};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::{
    env,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{error, info};

const JWKS_CACHE_TTL_SECS: u64 = 60 * 60;
const JWKS_KID_MISS_REFRESH_BACKOFF_SECS: u64 = 60;
const JWT_CLOCK_SKEW_SECS: i64 = 60;
const MAX_ADMIN_BEARER_TOKEN_BYTES: usize = 8 * 1024;
const MAX_LOG_VALUE_CHARS: usize = 120;
static ADMIN_AUTH_VERIFIER: OnceLock<Result<AdminClerkVerifier, String>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct AdminAuthContext {
    pub subject: String,
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminClerkClaims {
    sub: String,
    exp: i64,
    iat: i64,
    #[serde(default)]
    nbf: Option<i64>,
    iss: String,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    o: Option<LegacyOrgClaims>,
}

impl AdminClerkClaims {
    fn active_org_id(&self) -> Option<&str> {
        self.org_id
            .as_deref()
            .or_else(|| self.o.as_ref().and_then(|org| org.id.as_deref()))
    }
}

#[derive(Debug, Deserialize)]
struct LegacyOrgClaims {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct JwksKey {
    kty: String,
    kid: String,
    n: String,
    e: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwks {
    keys: Vec<JwksKey>,
}

#[derive(Debug, Deserialize)]
struct MinimalJwtHeader {
    alg: String,
    kid: String,
    #[serde(default)]
    crit: Option<Vec<String>>,
}

#[derive(Clone)]
struct AdminClerkVerifier {
    expected_issuer: Arc<str>,
    required_org_id: Option<Arc<str>>,
    allowed_azp: Arc<[String]>,
    jwks_cache: Arc<RwLock<Option<(Jwks, Instant)>>>,
    last_kid_miss_refresh: Arc<RwLock<Option<Instant>>>,
    http_client: Client,
}

impl AdminClerkVerifier {
    fn from_env() -> Result<Self, String> {
        let expected_issuer = load_expected_issuer()?;
        let required_org_id = env::var("ADMIN_CLERK_ORG_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| value.into());
        let allowed_azp = env::var("ADMIN_CLERK_AUTHORIZED_PARTIES")
            .ok()
            .map(|value| parse_csv_env(&value))
            .unwrap_or_default();

        if required_org_id.is_none() {
            info!("ADMIN_CLERK_ORG_ID is not set — admin auth will accept any valid JWT from the configured Clerk instance without org membership enforcement");
        }

        Ok(Self {
            expected_issuer: expected_issuer.into(),
            required_org_id,
            allowed_azp: allowed_azp.into(),
            jwks_cache: Arc::new(RwLock::new(None)),
            last_kid_miss_refresh: Arc::new(RwLock::new(None)),
            http_client: Client::new(),
        })
    }

    async fn fetch_jwks(&self, force_refresh: bool) -> Result<Jwks, String> {
        if !force_refresh {
            let cache = self.jwks_cache.read().await;
            if let Some((jwks, fetched_at)) = cache.as_ref() {
                if fetched_at.elapsed().as_secs() < JWKS_CACHE_TTL_SECS {
                    return Ok(jwks.clone());
                }
            }
        }

        let jwks_url = format!("{}/.well-known/jwks.json", self.expected_issuer);
        let response = self
            .http_client
            .get(&jwks_url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Clerk JWKS from {}: {}", jwks_url, e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Clerk JWKS endpoint returned {} for {}",
                response.status(),
                jwks_url
            ));
        }

        let jwks = response
            .json::<Jwks>()
            .await
            .map_err(|e| format!("Failed to parse Clerk JWKS response: {}", e))?;

        let mut cache = self.jwks_cache.write().await;
        *cache = Some((jwks.clone(), Instant::now()));

        Ok(jwks)
    }

    async fn fetch_jwks_after_kid_miss(&self) -> Result<Option<Jwks>, String> {
        let mut last_refresh = self.last_kid_miss_refresh.write().await;
        if last_refresh.as_ref().is_some_and(|instant| {
            instant.elapsed() < Duration::from_secs(JWKS_KID_MISS_REFRESH_BACKOFF_SECS)
        }) {
            return Ok(None);
        }

        let jwks = self.fetch_jwks(true).await?;
        *last_refresh = Some(Instant::now());
        Ok(Some(jwks))
    }

    async fn verify_token(&self, token: &str) -> Result<AdminAuthContext, String> {
        let (header_part, payload_part, signature_part) = split_jwt(token)?;
        let header = parse_minimal_jwt_header(header_part)?;
        validate_minimal_jwt_header(&header)?;

        let issuer = extract_unverified_issuer(token)
            .ok_or_else(|| "Clerk JWT is missing issuer claim".to_string())?;

        if issuer != self.expected_issuer.as_ref() {
            return Err(format!(
                "Clerk JWT issuer mismatch: expected {}, got {}",
                self.expected_issuer,
                log_safe_value(&issuer)
            ));
        }

        let jwks = self.fetch_jwks(false).await?;
        let refreshed_jwks = if find_jwks_key(&jwks, &header.kid).is_none() {
            self.fetch_jwks_after_kid_miss().await?
        } else {
            None
        };
        let key = find_jwks_key(&jwks, &header.kid)
            .or_else(|| refreshed_jwks.as_ref().and_then(|jwks| find_jwks_key(jwks, &header.kid)));
        let key = key.ok_or_else(|| {
            format!("Clerk JWKS does not contain key {}", log_safe_value(&header.kid))
        })?;

        let decoding_key = DecodingKey::from_rsa_components(&key.n, &key.e)
            .map_err(|e| format!("Failed to build Clerk decoding key: {}", e))?;

        let signing_input = format!("{}.{}", header_part, payload_part);
        let signature_is_valid = crypto::verify(
            signature_part,
            signing_input.as_bytes(),
            &decoding_key,
            Algorithm::RS256,
        )
        .map_err(|e| format!("Clerk JWT signature verification failed: {}", e))?;

        if !signature_is_valid {
            return Err("Clerk JWT signature is invalid".to_string());
        }

        let claims = decode_claims(payload_part)?;
        validate_claim_timestamps(&claims, get_current_timestamp() as i64)?;

        if normalize_url_like_value(&claims.iss) != self.expected_issuer.as_ref() {
            return Err(format!(
                "Clerk JWT issuer mismatch after validation: expected {}, got {}",
                self.expected_issuer,
                log_safe_value(&claims.iss)
            ));
        }

        if claims.sub.trim().is_empty() {
            return Err("Clerk JWT subject claim is empty".to_string());
        }

        validate_authorized_party(&claims, self.allowed_azp.as_ref())?;

        if let Some(ref required_org_id) = self.required_org_id {
            let org_id = claims
                .active_org_id()
                .ok_or_else(|| "Clerk JWT does not contain an active organization".to_string())?;
            if org_id != required_org_id.as_ref() {
                return Err(format!(
                    "Clerk JWT org mismatch: expected {}, got {}",
                    required_org_id,
                    log_safe_value(org_id)
                ));
            }
        }

        let _ = (claims.exp, claims.iat);

        let org_id = claims.active_org_id().map(str::to_string);

        Ok(AdminAuthContext {
            subject: claims.sub,
            org_id,
        })
    }
}

fn split_jwt(token: &str) -> Result<(&str, &str, &str), String> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or_else(|| "Clerk JWT is missing header".to_string())?;
    let payload = parts.next().ok_or_else(|| "Clerk JWT is missing payload".to_string())?;
    let signature = parts.next().ok_or_else(|| "Clerk JWT is missing signature".to_string())?;

    if parts.next().is_some() || header.is_empty() || payload.is_empty() || signature.is_empty() {
        return Err("Clerk JWT must contain exactly three non-empty parts".to_string());
    }

    Ok((header, payload, signature))
}

fn find_jwks_key<'a>(jwks: &'a Jwks, kid: &str) -> Option<&'a JwksKey> {
    jwks.keys
        .iter()
        .find(|key| key.kid == kid && key.kty.eq_ignore_ascii_case("RSA"))
}

fn parse_minimal_jwt_header(header_part: &str) -> Result<MinimalJwtHeader, String> {
    let header_bytes = base64_url_decode(header_part)
        .ok_or_else(|| "Failed to base64-decode Clerk JWT header".to_string())?;
    serde_json::from_slice(&header_bytes)
        .map_err(|e| format!("Failed to parse Clerk JWT header: {}", e))
}

fn validate_minimal_jwt_header(header: &MinimalJwtHeader) -> Result<(), String> {
    if header.alg != "RS256" {
        return Err(format!("Unsupported Clerk JWT algorithm: {}", log_safe_value(&header.alg)));
    }

    if header.kid.trim().is_empty() {
        return Err("Clerk JWT kid header is empty".to_string());
    }

    if header.crit.as_ref().is_some_and(|crit| !crit.is_empty()) {
        return Err("Clerk JWT contains unsupported critical headers".to_string());
    }

    Ok(())
}

fn decode_claims(payload_part: &str) -> Result<AdminClerkClaims, String> {
    let payload_bytes = base64_url_decode(payload_part)
        .ok_or_else(|| "Failed to base64-decode Clerk JWT payload".to_string())?;
    serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("Failed to parse Clerk JWT claims: {}", e))
}

fn validate_claim_timestamps(claims: &AdminClerkClaims, now: i64) -> Result<(), String> {
    if claims.exp < 0 || claims.exp.saturating_add(JWT_CLOCK_SKEW_SECS) < now {
        return Err("Clerk JWT is expired".to_string());
    }

    if let Some(nbf) = claims.nbf {
        if nbf > now.saturating_add(JWT_CLOCK_SKEW_SECS) {
            return Err("Clerk JWT is not valid yet".to_string());
        }
    }

    Ok(())
}

fn validate_authorized_party(
    claims: &AdminClerkClaims,
    allowed_azp: &[String],
) -> Result<(), String> {
    if allowed_azp.is_empty() {
        return Ok(());
    }

    let azp = claims
        .azp
        .as_deref()
        .ok_or_else(|| "Clerk JWT is missing azp claim".to_string())?;
    let azp = normalize_url_like_value(azp);
    if !allowed_azp.iter().any(|allowed| allowed == &azp) {
        return Err(format!("Clerk JWT azp is not allowed: {}", log_safe_value(&azp)));
    }

    Ok(())
}

fn load_expected_issuer() -> Result<String, String> {
    if let Some(issuer) = env::var("ADMIN_CLERK_FRONTEND_API")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(normalize_url_like_value(&issuer));
    }

    if let Some(issuer) = env::var("CLERK_FRONTEND_API")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(normalize_url_like_value(&issuer));
    }

    let publishable_key = env::var("CLERK_PUBLISHABLE_KEY")
        .map_err(|_| "CLERK_PUBLISHABLE_KEY must be set when CLERK_FRONTEND_API is missing".to_string())?;

    derive_issuer_from_publishable_key(&publishable_key).ok_or_else(|| {
        "Failed to derive Clerk issuer from CLERK_PUBLISHABLE_KEY; set CLERK_FRONTEND_API or ADMIN_CLERK_FRONTEND_API"
            .to_string()
    })
}

fn derive_issuer_from_publishable_key(publishable_key: &str) -> Option<String> {
    let mut parts = publishable_key.splitn(3, '_');
    if parts.next()? != "pk" {
        return None;
    }
    parts.next()?;
    let encoded_host = parts.next()?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded_host).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let host = decoded.trim_end_matches('$').trim();
    if host.is_empty() {
        return None;
    }

    Some(normalize_url_like_value(host))
}

fn extract_unverified_issuer(token: &str) -> Option<String> {
    let payload_part = token.split('.').nth(1)?;
    let payload_bytes = base64_url_decode(payload_part)?;
    let payload_json: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    let issuer = payload_json.get("iss")?.as_str()?;
    Some(normalize_url_like_value(issuer))
}

fn base64_url_decode(input: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| URL_SAFE.decode(input))
        .ok()
}

fn normalize_url_like_value(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    }
}

fn parse_csv_env(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(normalize_url_like_value)
        .collect()
}

fn log_safe_value(value: &str) -> String {
    let mut safe = String::new();
    let mut truncated = false;

    for (idx, ch) in value.chars().enumerate() {
        if idx >= MAX_LOG_VALUE_CHARS {
            truncated = true;
            break;
        }

        if ch.is_control() {
            safe.push('?');
        } else {
            safe.push(ch);
        }
    }

    if truncated {
        safe.push_str("...");
    }

    safe
}

fn unauthorized_response(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "unauthorized",
            "message": message
        })),
    )
}

fn internal_error_response(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "config_error",
            "message": message
        })),
    )
}

pub fn admin_auth_bypass_allowed(environment: &str, bypass_enabled: bool) -> bool {
    if !bypass_enabled {
        return false;
    }
    let env_lower = environment.trim().to_ascii_lowercase();
    matches!(env_lower.as_str(), "development" | "dev" | "local")
}

/// Clerk admin authentication middleware
/// Validates that request is from Tyde's internal Clerk organization
pub async fn admin_auth_middleware(
    mut request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
    let bypass_enabled = crate::config::parse_bool_env("BYPASS_ADMIN_AUTH", false).unwrap_or(false);

    if admin_auth_bypass_allowed(&environment, bypass_enabled) {
        let admin = AdminAuthContext {
            subject: "mock_admin_user".to_string(),
            org_id: Some("mock_org".to_string()),
        };
        request.extensions_mut().insert(admin.clone());
        let mut response = next.run(request).await;
        response.extensions_mut().insert(admin);
        return Ok(response);
    }
    let authorization = match request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
    {
        Some(value) => value,
        _ => {
            error!("Admin endpoint accessed without auth token");
            return Err(unauthorized_response(
                "Admin endpoints require authentication",
            ));
        }
    };

    if authorization.len() > "Bearer ".len() + MAX_ADMIN_BEARER_TOKEN_BYTES {
        error!("Admin auth rejected oversized Authorization header");
        return Err(unauthorized_response("Invalid admin session"));
    }

    let token = match authorization.strip_prefix("Bearer ") {
        Some(token) if !token.trim().is_empty() => token,
        _ => {
            error!("Admin endpoint accessed without bearer auth token");
            return Err(unauthorized_response(
                "Admin endpoints require authentication",
            ));
        }
    };

    let verifier = match ADMIN_AUTH_VERIFIER.get_or_init(AdminClerkVerifier::from_env) {
        Ok(verifier) => verifier,
        Err(message) => {
            error!("Admin auth configuration error: {}", message);
            return Err(internal_error_response(
                "Admin authentication is not configured",
            ));
        }
    };

    let admin = match verifier.verify_token(token).await {
        Ok(admin) => admin,
        Err(message) => {
            error!("Admin auth rejected request: {}", message);
            return Err(unauthorized_response("Invalid admin session"));
        }
    };

    request.extensions_mut().insert(admin.clone());

    let mut response = next.run(request).await;
    response.extensions_mut().insert(admin);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use serde_json::json;

    use super::{
        derive_issuer_from_publishable_key, parse_csv_env, parse_minimal_jwt_header,
        validate_authorized_party, validate_claim_timestamps, validate_minimal_jwt_header,
        AdminClerkClaims, LegacyOrgClaims,
    };

    fn jwt_part(value: serde_json::Value) -> String {
        URL_SAFE_NO_PAD.encode(value.to_string())
    }

    fn valid_claims() -> AdminClerkClaims {
        AdminClerkClaims {
            sub: "user_2OX4Z9LfHpKq3rBw".to_string(),
            exp: 1_735_689_600,
            iat: 1_735_686_000,
            nbf: None,
            iss: "https://test-bridge-admin.clerk.accounts.dev".to_string(),
            azp: None,
            org_id: None,
            o: None,
        }
    }

    #[test]
    fn derives_issuer_from_publishable_key() {
        let issuer = derive_issuer_from_publishable_key(
            "pk_test_dGVzdC1icmlkZ2UtYWRtaW4uY2xlcmsuYWNjb3VudHMuZGV2JA",
        );

        assert_eq!(
            issuer.as_deref(),
            Some("https://test-bridge-admin.clerk.accounts.dev")
        );
    }

    #[test]
    fn active_org_id_prefers_top_level_and_falls_back_to_legacy_shape() {
        let top_level = AdminClerkClaims {
            sub: "user_2OX4Z9LfHpKq3rBw".to_string(),
            exp: 1735689600,
            iat: 1735686000,
            nbf: None,
            iss: "https://test-bridge-admin.clerk.accounts.dev".to_string(),
            azp: None,
            org_id: Some("org_2QXw7YnKzR4p".to_string()),
            o: Some(LegacyOrgClaims {
                id: Some("org_1PvM3cLhQsBz".to_string()),
            }),
        };
        let legacy_only = AdminClerkClaims {
            sub: "user_2OX4Z9LfHpKq3rBw".to_string(),
            exp: 1735689600,
            iat: 1735686000,
            nbf: None,
            iss: "https://test-bridge-admin.clerk.accounts.dev".to_string(),
            azp: None,
            org_id: None,
            o: Some(LegacyOrgClaims {
                id: Some("org_1PvM3cLhQsBz".to_string()),
            }),
        };

        assert_eq!(top_level.active_org_id(), Some("org_2QXw7YnKzR4p"));
        assert_eq!(legacy_only.active_org_id(), Some("org_1PvM3cLhQsBz"));
    }

    #[test]
    fn authorized_party_requires_azp_when_allowlist_is_configured() {
        let claims = valid_claims();
        let allowed = vec!["https://admin.tyde.app".to_string()];

        let err = validate_authorized_party(&claims, &allowed).unwrap_err();

        assert_eq!(err, "Clerk JWT is missing azp claim");
    }

    #[test]
    fn authorized_party_accepts_normalized_allowed_azp() {
        let mut claims = valid_claims();
        claims.azp = Some("https://admin.tyde.app/".to_string());
        let allowed = vec!["https://admin.tyde.app".to_string()];

        validate_authorized_party(&claims, &allowed).unwrap();
    }

    #[test]
    fn authorized_party_rejects_unlisted_azp() {
        let mut claims = valid_claims();
        claims.azp = Some("https://evil.example".to_string());
        let allowed = vec!["https://admin.tyde.app".to_string()];

        let err = validate_authorized_party(&claims, &allowed).unwrap_err();

        assert_eq!(err, "Clerk JWT azp is not allowed: https://evil.example");
    }

    #[test]
    fn parse_csv_env_ignores_blank_entries_before_normalizing_urls() {
        let values = parse_csv_env(" https://admin.tyde.app/, , http://localhost:3000 ,, ");

        assert_eq!(values, vec![
            "https://admin.tyde.app".to_string(),
            "http://localhost:3000".to_string(),
        ]);
    }

    #[test]
    fn header_parser_accepts_numeric_unknown_fields() {
        let header = parse_minimal_jwt_header(&jwt_part(json!({
            "alg": "RS256",
            "kid": "test-key",
            "some_numeric_extra": 1782121102
        })))
        .unwrap();

        assert_eq!(header.alg, "RS256");
        assert_eq!(header.kid, "test-key");
    }

    #[test]
    fn header_parser_rejects_numeric_kid() {
        let err = parse_minimal_jwt_header(&jwt_part(json!({
            "alg": "RS256",
            "kid": 1782121102
        })))
        .unwrap_err();

        assert!(err.contains("Failed to parse Clerk JWT header"));
    }

    #[test]
    fn header_validation_rejects_wrong_algorithm() {
        let header = parse_minimal_jwt_header(&jwt_part(json!({
            "alg": "HS256",
            "kid": "test-key"
        })))
        .unwrap();

        let err = validate_minimal_jwt_header(&header).unwrap_err();

        assert_eq!(err, "Unsupported Clerk JWT algorithm: HS256");
    }

    #[test]
    fn header_validation_rejects_non_empty_crit() {
        let header = parse_minimal_jwt_header(&jwt_part(json!({
            "alg": "RS256",
            "kid": "test-key",
            "crit": ["custom"]
        })))
        .unwrap();

        let err = validate_minimal_jwt_header(&header).unwrap_err();

        assert_eq!(err, "Clerk JWT contains unsupported critical headers");
    }

    #[test]
    fn claim_timestamp_validation_rejects_expired_tokens() {
        let mut claims = valid_claims();
        claims.exp = 1_000;

        let err = validate_claim_timestamps(&claims, 1_061).unwrap_err();

        assert_eq!(err, "Clerk JWT is expired");
    }

    #[test]
    fn claim_timestamp_validation_rejects_future_nbf() {
        let mut claims = valid_claims();
        claims.nbf = Some(1_061);

        let err = validate_claim_timestamps(&claims, 1_000).unwrap_err();

        assert_eq!(err, "Clerk JWT is not valid yet");
    }

    #[test]
    fn test_admin_auth_bypass_allowed() {
        use super::admin_auth_bypass_allowed;

        // bypass_enabled = false should always be false
        assert!(!admin_auth_bypass_allowed("development", false));
        assert!(!admin_auth_bypass_allowed("production", false));

        // bypass_enabled = true and dev/local environments should be true
        assert!(admin_auth_bypass_allowed("development", true));
        assert!(admin_auth_bypass_allowed("dev", true));
        assert!(admin_auth_bypass_allowed("local", true));
        assert!(admin_auth_bypass_allowed(" DEVELOPMENT ", true));
        assert!(admin_auth_bypass_allowed("Dev", true));

        // production and prod variants should be false
        assert!(!admin_auth_bypass_allowed("production", true));
        assert!(!admin_auth_bypass_allowed("prod", true));
        assert!(!admin_auth_bypass_allowed(" PROD ", true));

        // unknown/staging should be false
        assert!(!admin_auth_bypass_allowed("staging", true));
        assert!(!admin_auth_bypass_allowed("demo", true));
        assert!(!admin_auth_bypass_allowed("unknown", true));
    }
}

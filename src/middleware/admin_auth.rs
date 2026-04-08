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
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::{
    env,
    sync::{Arc, OnceLock},
    time::Instant,
};
use tokio::sync::RwLock;
use tracing::error;

const JWKS_CACHE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
static ADMIN_AUTH_VERIFIER: OnceLock<Result<AdminClerkVerifier, String>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct AdminClerkClaims {
    sub: String,
    exp: i64,
    iat: i64,
    iss: String,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    sts: Option<String>,
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

#[derive(Clone)]
struct AdminClerkVerifier {
    expected_issuer: Arc<str>,
    required_org_id: Arc<str>,
    allowed_azp: Arc<[String]>,
    jwks_cache: Arc<RwLock<Option<(Jwks, Instant)>>>,
    http_client: Client,
}

impl AdminClerkVerifier {
    fn from_env() -> Result<Self, String> {
        let expected_issuer = load_expected_issuer()?;
        let required_org_id = env::var("ADMIN_CLERK_ORG_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "ADMIN_CLERK_ORG_ID must be set for admin auth".to_string())?;
        let allowed_azp = env::var("ADMIN_CLERK_AUTHORIZED_PARTIES")
            .ok()
            .map(|value| parse_csv_env(&value))
            .unwrap_or_default();

        Ok(Self {
            expected_issuer: expected_issuer.into(),
            required_org_id: required_org_id.into(),
            allowed_azp: allowed_azp.into(),
            jwks_cache: Arc::new(RwLock::new(None)),
            http_client: Client::new(),
        })
    }

    async fn fetch_jwks(&self) -> Result<Jwks, String> {
        {
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

    async fn verify_token(&self, token: &str) -> Result<(), String> {
        let header = decode_header(token)
            .map_err(|e| format!("Failed to decode Clerk JWT header: {}", e))?;

        if header.alg != Algorithm::RS256 {
            return Err(format!("Unsupported Clerk JWT algorithm: {:?}", header.alg));
        }

        let kid = header
            .kid
            .ok_or_else(|| "Clerk JWT is missing kid header".to_string())?;
        let issuer = extract_unverified_issuer(token)
            .ok_or_else(|| "Clerk JWT is missing issuer claim".to_string())?;

        if issuer != self.expected_issuer.as_ref() {
            return Err(format!(
                "Clerk JWT issuer mismatch: expected {}, got {}",
                self.expected_issuer, issuer
            ));
        }

        let jwks = self.fetch_jwks().await?;
        let key = jwks
            .keys
            .iter()
            .find(|key| key.kid == kid && key.kty.eq_ignore_ascii_case("RSA"))
            .ok_or_else(|| format!("Clerk JWKS does not contain key {}", kid))?;

        let decoding_key = DecodingKey::from_rsa_components(&key.n, &key.e)
            .map_err(|e| format!("Failed to build Clerk decoding key: {}", e))?;
        let token_data = decode::<AdminClerkClaims>(
            token,
            &decoding_key,
            &Validation::new(Algorithm::RS256),
        )
        .map_err(|e| format!("Clerk JWT validation failed: {}", e))?;

        let claims = token_data.claims;
        if normalize_url_like_value(&claims.iss) != self.expected_issuer.as_ref() {
            return Err(format!(
                "Clerk JWT issuer mismatch after validation: expected {}, got {}",
                self.expected_issuer, claims.iss
            ));
        }

        if claims.sub.trim().is_empty() {
            return Err("Clerk JWT subject claim is empty".to_string());
        }

        if claims.sts.as_deref() == Some("pending") {
            return Err("Clerk JWT session is pending organization enrollment".to_string());
        }

        if !self.allowed_azp.is_empty() {
            if let Some(azp) = claims.azp.as_deref() {
                let azp = normalize_url_like_value(azp);
                if !self.allowed_azp.iter().any(|allowed| allowed == &azp) {
                    return Err(format!("Clerk JWT azp is not allowed: {}", azp));
                }
            }
        }

        let org_id = claims
            .active_org_id()
            .ok_or_else(|| "Clerk JWT does not contain an active organization".to_string())?;
        if org_id != self.required_org_id.as_ref() {
            return Err(format!(
                "Clerk JWT org mismatch: expected {}, got {}",
                self.required_org_id, org_id
            ));
        }

        let _ = (claims.exp, claims.iat);

        Ok(())
    }
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
        .map(normalize_url_like_value)
        .filter(|part| !part.is_empty())
        .collect()
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

/// Clerk admin authentication middleware
/// Validates that request is from Tyde's internal Clerk organization
pub async fn admin_auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let token = match request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        Some(token) if !token.trim().is_empty() => token,
        _ => {
            error!("Admin endpoint accessed without auth token");
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

    if let Err(message) = verifier.verify_token(token).await {
        error!("Admin auth rejected request: {}", message);
        return Err(unauthorized_response("Invalid admin session"));
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::{derive_issuer_from_publishable_key, AdminClerkClaims, LegacyOrgClaims};

    #[test]
    fn derives_issuer_from_publishable_key() {
        let issuer = derive_issuer_from_publishable_key(
            "pk_test_bWFpbi1jaXZldC04LmNsZXJrLmFjY291bnRzLmRldiQ",
        );

        assert_eq!(
            issuer.as_deref(),
            Some("https://main-civet-8.clerk.accounts.dev")
        );
    }

    #[test]
    fn active_org_id_prefers_top_level_and_falls_back_to_legacy_shape() {
        let top_level = AdminClerkClaims {
            sub: "user_123".to_string(),
            exp: 1,
            iat: 1,
            iss: "https://example.clerk.accounts.dev".to_string(),
            azp: None,
            sts: None,
            org_id: Some("org_top".to_string()),
            o: Some(LegacyOrgClaims {
                id: Some("org_legacy".to_string()),
            }),
        };
        let legacy_only = AdminClerkClaims {
            sub: "user_123".to_string(),
            exp: 1,
            iat: 1,
            iss: "https://example.clerk.accounts.dev".to_string(),
            azp: None,
            sts: None,
            org_id: None,
            o: Some(LegacyOrgClaims {
                id: Some("org_legacy".to_string()),
            }),
        };

        assert_eq!(top_level.active_org_id(), Some("org_top"));
        assert_eq!(legacy_only.active_org_id(), Some("org_legacy"));
    }
}

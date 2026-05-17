use std::time::{SystemTime, UNIX_EPOCH, Duration};
use anyhow::Result;
use jsonwebtoken::{decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use crate::error::AppError;
use crate::utils::redact_with_prefix;
use backoff::ExponentialBackoff;
use backoff::future::retry;

#[derive(Debug, Deserialize)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    // other fields ignored
}

#[derive(Debug, Serialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    exp: u64,
    iat: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    // expires_in: i32,
}

/// Google's public key set response
#[derive(Debug, Deserialize, Clone)]
struct GoogleCerts {
    keys: Vec<JsonWebKey>,
}

/// Individual JSON Web Key from Google
#[derive(Debug, Deserialize, Clone)]
struct JsonWebKey {
    #[allow(dead_code)]
    kty: String,        // Key type (RSA)
    #[serde(rename = "use")]
    #[allow(dead_code)]
    use_: Option<String>, // "sig" for signing
    kid: String,        // Key ID
    n: String,          // Modulus (base64url)
    e: String,          // Exponent (base64url)
    #[allow(dead_code)]
    alg: Option<String>, // Algorithm
}

/// JWT claims from Google Pub/Sub
#[derive(Debug, Deserialize, Serialize)]
struct PubSubClaims {
    iss: String,        // Issuer
    aud: String,        // Audience (webhook URL)
    exp: u64,           // Expiration time
    iat: u64,           // Issued at
    email: Option<String>, // Service account email
}

/// Cached Google public keys with expiration
#[derive(Debug, Clone)]
struct CachedKeys {
    keys: Vec<JsonWebKey>,
    expires_at: SystemTime,
}

#[derive(Clone)]
pub struct GooglePlayClient {
    client: Client,
    service_account: std::sync::Arc<ServiceAccount>,
    cached_keys: std::sync::Arc<tokio::sync::RwLock<Option<CachedKeys>>>,
    pub verify_aud: bool,
    pub pub_sub_audience: String,
    skip_rsa: bool,
}

impl GooglePlayClient {
    pub fn new(service_account_path: &str) -> Result<Self> {
        Self::with_config(service_account_path, false, String::new(), false)
    }

    pub fn with_config(service_account_path: &str, verify_aud: bool, pub_sub_audience: String, skip_rsa: bool) -> Result<Self> {
        let content = fs::read_to_string(service_account_path)?;
        let service_account: ServiceAccount = serde_json::from_str(&content)?;

        Ok(Self {
            client: Client::new(),
            service_account: std::sync::Arc::new(service_account),
            cached_keys: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            verify_aud,
            pub_sub_audience,
            skip_rsa,
        })
    }

    /// Override audience verification setting for testing
    /// Returns a clone with modified verify_aud setting
    #[allow(dead_code)]
    pub fn with_audience_override(&self, verify_aud: bool) -> Self {
        Self {
            client: self.client.clone(),
            service_account: self.service_account.clone(),
            cached_keys: self.cached_keys.clone(),
            verify_aud,
            pub_sub_audience: self.pub_sub_audience.clone(),
            skip_rsa: self.skip_rsa,
        }
    }

    async fn get_access_token(&self) -> Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let claims = Claims {
            iss: self.service_account.client_email.clone(),
            scope: "https://www.googleapis.com/auth/androidpublisher".to_string(),
            aud: "https://oauth2.googleapis.com/token".to_string(),
            exp: now + 3600,
            iat: now,
        };

        let header = Header::new(Algorithm::RS256);
        let key = EncodingKey::from_rsa_pem(self.service_account.private_key.as_bytes())?;
        let jwt = encode(&header, &claims, &key)?;

        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ];

        let res = self.client
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await?;

        if !res.status().is_success() {
             return Err(anyhow::anyhow!("Failed to get access token: status {}", res.status()));
        }

        let token_res: TokenResponse = res.json().await?;
        Ok(token_res.access_token)
    }

    pub async fn get_subscription(
        &self,
        package_name: &str,
        _subscription_id: &str, // Deprecated in V2 GET, but kept for interface consistency
        token: &str,
    ) -> Result<super::models::SubscriptionPurchaseV2> {
        tracing::debug!("GooglePlayClient: get_subscription (v2) - package: {}, token: {}", package_name, redact_with_prefix(token));
        
        let access_token = self.get_access_token().await?;

        // V2 API: GET /androidpublisher/v3/applications/{packageName}/purchases/subscriptionsv2/tokens/{token}
        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptionsv2/tokens/{}",
            package_name, token
        );
        tracing::debug!("GooglePlayClient: calling API endpoint for subscription");

        let res = self.client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if !res.status().is_success() {
             let status = res.status();
             let text = res.text().await?;
             tracing::error!("GooglePlayClient: API error - package: {}, status: {}", package_name, status);
             tracing::debug!("Response: {}", text);
             tracing::debug!(target: "BPT-RAW", "GooglePlay Error Response - get_subscription (v2): status={}, body={}", status, text);
             return Err(anyhow::anyhow!("Failed to get subscription for package {}, token {}: {}", package_name, redact_with_prefix(token), text));
        }

        let text = res.text().await?;
        tracing::debug!(target: "BPT-RAW", "GooglePlay Raw Response - get_subscription (v2): {}", text);

        let purchase: super::models::SubscriptionPurchaseV2 = serde_json::from_str(&text).map_err(|e| {
             anyhow::anyhow!("Failed to parse subscription response: {} | Raw body: {}", e, text)
        })?;
        let effective_expiry = purchase
            .line_items
            .first()
            .and_then(|item| item.expiry_time.as_deref())
            .or(purchase.expiry_time.as_deref());
        tracing::info!(
            "GooglePlay subscription retrieved: state: {:?}, expiry: {:?}",
            purchase.subscription_state,
            effective_expiry
        );
        Ok(purchase)
    }

    pub async fn get_product(
        &self,
        package_name: &str,
        product_id: &str,
        token: &str,
    ) -> Result<super::models::ProductPurchase> {
        tracing::debug!("GooglePlayClient: get_product - package: {}, product_id: {}", package_name, product_id);

        let access_token = self.get_access_token().await?;

        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/products/{}/tokens/{}",
            package_name, product_id, token
        );
        tracing::debug!("GooglePlayClient: calling API endpoint for product");

        let res = self.client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if !res.status().is_success() {
             let status = res.status();
             let text = res.text().await?;
             tracing::error!("GooglePlayClient: API error - package: {}, product: {}, status: {}, response: {}", package_name, product_id, status, text);
             tracing::debug!(target: "BPT-RAW", "GooglePlay Error Response - get_product: status={}, body={}", status, text);
             return Err(anyhow::anyhow!("Failed to get product: {}, response: {}", url, text));
        }

        let text = res.text().await?;
        tracing::debug!(target: "BPT-RAW", "GooglePlay Raw Response - get_product: {}", text);

        let purchase: super::models::ProductPurchase = serde_json::from_str(&text).map_err(|e| {
             anyhow::anyhow!("Failed to parse product response: {} | Raw body: {}", e, text)
        })?;
        tracing::info!("GooglePlay product retrieved: purchase_state: {}", purchase.purchase_state);
        Ok(purchase)
    }

    pub async fn get_order_amount_cents(
        &self,
        package_name: &str,
        order_id: &str,
    ) -> Result<Option<i32>> {
        tracing::debug!("GooglePlayClient: get_order_amount_cents - package: {}, order_id: {}", package_name, order_id);

        let access_token = self.get_access_token().await?;

        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/orders/{}",
            package_name, order_id
        );

        let res = self.client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await?;
            tracing::error!(
                "GooglePlayClient: Orders API error - package: {}, order_id: {}, status: {}, response: {}",
                package_name,
                order_id,
                status,
                text
            );
            return Err(anyhow::anyhow!("Failed to get order: {}, response: {}", url, text));
        }

        let order: serde_json::Value = res.json().await?;
        Ok(order_amount_cents_from_payload(&order))
    }

    pub async fn cancel_subscription(
        &self,
        package_name: &str,
        subscription_id: &str,
        token: &str,
    ) -> Result<()> {
        tracing::info!("GooglePlayClient: cancel_subscription - package: {}, subscription_id: {}, token: {}", package_name, subscription_id, redact_with_prefix(token));
        
        let access_token = self.get_access_token().await?;

        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptionsv2/{}/tokens/{}:cancel",
            package_name, subscription_id, token
        );

        // Request body with cancellation type: user-initiated (stop renewal)
        // User retains access until current billing period ends
        let body = serde_json::json!({
            "cancellationType": "USER_REQUESTED_STOP_RENEWAL"
        });

        let res = self.client
            .post(&url)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await?;

        if !res.status().is_success() {
             let status = res.status();
             let text = res.text().await?;
             tracing::error!("GooglePlayClient: Failed to cancel subscription - package: {}, subscription: {}, status: {}, response: {}", package_name, subscription_id, status, text);
             return Err(anyhow::anyhow!("Failed to cancel subscription: status {}, response: {}", status, text));
        }

        let text = res.text().await?;
        tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - cancel_subscription: {}", text);

        tracing::info!("GooglePlay subscription cancelled successfully");
        Ok(())
    }

    pub async fn acknowledge_subscription(
        &self,
        package_name: &str,
        subscription_id: &str,
        token: &str,
    ) -> Result<()> {
        tracing::info!("GooglePlayClient: acknowledge_subscription - package: {}, subscription_id: {}, token: {}", package_name, subscription_id, redact_with_prefix(token));

        // Exponential backoff for transient errors
        let backoff = ExponentialBackoff {
            initial_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(10),
            multiplier: 2.0,
            max_elapsed_time: Some(Duration::from_secs(30)),
            ..Default::default()
        };

        let result = retry(backoff, || async {
            let access_token = self.get_access_token().await
                .map_err(|e| backoff::Error::transient(anyhow::anyhow!("Failed to get access token: {}", e)))?;

            let url = format!(
                "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptions/{}/tokens/{}:acknowledge",
                package_name, subscription_id, token
            );

            // Empty body for acknowledge
            let body = serde_json::json!({});

            let res = self.client
                .post(&url)
                .bearer_auth(access_token)
                .json(&body)
                .send()
                .await
                .map_err(|e| backoff::Error::transient(anyhow::anyhow!("HTTP error: {}", e)))?;

            let status = res.status();

            let text = res.text().await
                .unwrap_or_else(|_| "(failed to read response body)".to_string());

            if status.is_success() {
                tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - acknowledge_subscription: {}", text);
                return Ok(());
            }

            // Distinguish between HTTP errors
            // 400 = already acknowledged (success) or invalid state
            // 404 = token not found or expired (permanent failure)
            // 5xx = transient errors (retry)

            if status == reqwest::StatusCode::BAD_REQUEST {
                if text.contains("already acknowledged") {
                    tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - acknowledge_subscription (already acknowledged): {}", text);
                    tracing::info!("GooglePlayClient: Subscription already acknowledged (ignoring error)");
                    return Ok(());
                }
                // Other 400 errors are permanent failures
                return Err(backoff::Error::permanent(anyhow::anyhow!("Failed to acknowledge subscription (400): {}", text)));
                }

                if status == reqwest::StatusCode::NOT_FOUND {
                tracing::warn!("GooglePlayClient: Subscription token not found or expired (404)");
                return Err(backoff::Error::permanent(anyhow::anyhow!("Subscription acknowledgement failed: token not found or expired (404)")));
                }

                // Server errors (5xx) are transient - retry
            if status.is_server_error() {
                tracing::error!("GooglePlayClient: Server error acknowledging subscription ({}), will retry", status);
                return Err(backoff::Error::transient(anyhow::anyhow!("Server error {}, will retry", status)));
            }

            // Unknown error - treat as permanent
            tracing::error!("GooglePlayClient: Failed to acknowledge subscription - package: {}, subscription: {}, status: {}, response: {}", package_name, subscription_id, status, text);
            Err(backoff::Error::permanent(anyhow::anyhow!("Failed to acknowledge subscription: status {}, response: {}", status, text)))
        }).await;

        match result {
            Ok(()) => {
                tracing::info!("GooglePlay subscription acknowledged successfully");
                Ok(())
            }
            Err(e) => {
                tracing::error!("GooglePlay subscription acknowledgement failed: {}", e);
                Err(e)
            }
        }
    }

    pub async fn acknowledge(
        &self,
        package_name: &str,
        product_id: &str,
        token: &str,
    ) -> Result<()> {
        tracing::info!("GooglePlayClient: acknowledge - package: {}, product_id: {}, token: {}", package_name, product_id, redact_with_prefix(token));

        // Exponential backoff for transient errors
        let backoff = ExponentialBackoff {
            initial_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(10),
            multiplier: 2.0,
            max_elapsed_time: Some(Duration::from_secs(30)),
            ..Default::default()
        };

        let result = retry(backoff, || async {
            let access_token = self.get_access_token().await
                .map_err(|e| backoff::Error::transient(anyhow::anyhow!("Failed to get access token: {}", e)))?;

            let url = format!(
                "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/products/{}/tokens/{}:acknowledge",
                package_name, product_id, token
            );

            // Empty body for acknowledge
            let body = serde_json::json!({});

            let res = self.client
                .post(&url)
                .bearer_auth(access_token)
                .json(&body)
                .send()
                .await
                .map_err(|e| backoff::Error::transient(anyhow::anyhow!("HTTP error: {}", e)))?;

            let status = res.status();

            let text = res.text().await
                .unwrap_or_else(|_| "(failed to read response body)".to_string());

            if status.is_success() {
                tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - acknowledge: {}", text);
                return Ok(());
            }

            if status == reqwest::StatusCode::BAD_REQUEST {
                if text.contains("already acknowledged") {
                    tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - acknowledge (already acknowledged): {}", text);
                    tracing::info!("GooglePlayClient: Purchase already acknowledged (ignoring error)");
                    return Ok(());
                }
                return Err(backoff::Error::permanent(anyhow::anyhow!("Failed to acknowledge (400): {}", text)));
            }

            if status == reqwest::StatusCode::NOT_FOUND {
                tracing::warn!("GooglePlayClient: Purchase token not found or expired (404)");
                return Err(backoff::Error::permanent(anyhow::anyhow!("Acknowledgement failed: token not found or expired (404)")));
            }

            if status.is_server_error() {
                tracing::error!("GooglePlayClient: Server error acknowledging ({}), will retry", status);
                return Err(backoff::Error::transient(anyhow::anyhow!("Server error {}, will retry", status)));
            }

            tracing::error!("GooglePlayClient: Failed to acknowledge - package: {}, product_id: {}, status: {}, response: {}", package_name, product_id, status, text);
            Err(backoff::Error::permanent(anyhow::anyhow!("Failed to acknowledge: status {}, response: {}", status, text)))
        }).await;

        match result {
            Ok(()) => {
                tracing::info!("GooglePlay purchase acknowledged successfully");
                Ok(())
            }
            Err(e) => {
                tracing::error!("Error acknowledging purchase: {}", e);
                Err(e)
            }
        }
    }

    /// Fetch Google's public keys and cache them with expiration
    async fn get_google_public_keys(&self) -> Result<Vec<JsonWebKey>, AppError> {
        // Check cache first
        {
            let cached = self.cached_keys.read().await;
            if let Some(cached) = cached.as_ref() {
                if cached.expires_at > SystemTime::now() {
                    tracing::debug!("Using cached Google public keys");
                    return Ok(cached.keys.clone());
                }
            }
        }

        // Fetch fresh keys from Google
        tracing::debug!("Fetching Google public keys from https://www.googleapis.com/oauth2/v3/certs");
        let response = self.client
            .get("https://www.googleapis.com/oauth2/v3/certs")
            .send()
            .await
            .map_err(|e| {
                let err = format!("Failed to fetch Google public keys: {}", e);
                AppError::WebhookSignatureVerificationFailed(err)
            })?;

        if !response.status().is_success() {
            let err = format!("Google public keys endpoint returned status: {}", response.status());
            tracing::error!("{}", err);
            return Err(AppError::WebhookSignatureVerificationFailed(err));
        }

        // Parse cache-control header to determine TTL
        let cache_control = response
            .headers()
            .get("cache-control")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let max_age_secs = Self::extract_max_age(&cache_control).unwrap_or(3600); // Default 1 hour

        let certs: GoogleCerts = response.json().await.map_err(|e| {
            let err = format!("Failed to parse Google certificates response: {}", e);
            AppError::WebhookSignatureVerificationFailed(err)
        })?;

        tracing::debug!(
            "Fetched {} Google public keys, cache TTL: {}s",
            certs.keys.len(),
            max_age_secs
        );

        // Update cache
        let expires_at = SystemTime::now() + Duration::from_secs(max_age_secs);
        *self.cached_keys.write().await = Some(CachedKeys {
            keys: certs.keys.clone(),
            expires_at,
        });

        Ok(certs.keys)
    }

    /// Extract max-age from Cache-Control header
    fn extract_max_age(cache_control: &str) -> Option<u64> {
        cache_control
            .split(',')
            .find_map(|part| {
                let part = part.trim();
                if part.starts_with("max-age=") {
                    part.strip_prefix("max-age=")
                        .and_then(|s| s.parse().ok())
                } else {
                    None
                }
            })
    }

    /// Verify JWT signature using JWK components
    fn verify_jwt_with_jwk(token: &str, jwk: &JsonWebKey, skip_rsa: bool) -> Result<PubSubClaims, AppError> {
        use base64::{Engine as _, engine::general_purpose};

        // Skip RSA verification if configured (dev/testing only)
        if skip_rsa {
            tracing::warn!("RSA signature verification SKIPPED (GOOGLE_SKIP_RSA_VERIFICATION=true) - do not use in production!");
            
            // Manually decode JWT without signature verification, but still check exp/iss
            let parts: Vec<&str> = token.split('.').collect();
            if parts.len() != 3 {
                return Err(AppError::WebhookSignatureVerificationFailed(
                    "Invalid JWT format: expected 3 parts".to_string()
                ));
            }

            let payload_b64 = parts[1];
            let payload_bytes = general_purpose::URL_SAFE_NO_PAD
                .decode(payload_b64)
                .map_err(|e| {
                    AppError::WebhookSignatureVerificationFailed(
                        format!("Failed to decode JWT payload: {}", e)
                    )
                })?;

            let claims: PubSubClaims = serde_json::from_slice(&payload_bytes)
                .map_err(|e| {
                    AppError::WebhookSignatureVerificationFailed(
                        format!("Failed to parse JWT claims: {}", e)
                    )
                })?;

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| AppError::WebhookSignatureVerificationFailed(
                    format!("System time error: {}", e)
                ))?
                .as_secs();

            if claims.exp <= now {
                return Err(AppError::WebhookSignatureVerificationFailed(
                    format!("JWT token expired (exp: {}, now: {})", claims.exp, now)
                ));
            }

            if claims.iss != "https://accounts.google.com" {
                return Err(AppError::WebhookSignatureVerificationFailed(
                    format!("Invalid issuer: {}", claims.iss)
                ));
            }

            return Ok(claims);
        }

        // Production path: verify signature with jsonwebtoken + check audience separately
        let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
            .map_err(|e| {
                AppError::WebhookSignatureVerificationFailed(
                    format!("Failed to construct RSA decoding key: {}", e)
                )
            })?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&["https://accounts.google.com"]);
        validation.validate_exp = true;
        validation.validate_aud = false;

        let verified = decode::<PubSubClaims>(token, &decoding_key, &validation)
            .map_err(|e| {
                AppError::WebhookSignatureVerificationFailed(
                    format!("JWT signature verification failed: {}", e)
                )
            })?;

        tracing::info!(
            "JWT verified: issuer={}, exp={}, aud={}",
            verified.claims.iss, verified.claims.exp, verified.claims.aud
        );

        Ok(verified.claims)
    }

    /// Verify Google Pub/Sub message authentication header.
    /// 
    /// Issue #6: Webhook Signature Verification
    /// 
    /// Pub/Sub messages can optionally include an Authorization header with a signed JWT.
    /// Google Cloud Pub/Sub can be configured to:
    /// 1. Push to authenticated endpoints (recommended)
    /// 2. Include Authorization Bearer token for client-side verification
    /// 
    /// This validates the authorization token is a valid JWT from Google.
    /// For production, ensure Pub/Sub is configured with authentication enabled
    /// in Google Cloud Console → Pub/Sub → Subscription settings.
    pub async fn verify_pubsub_signature(&self, authorization_header: &str) -> Result<bool> {
        if authorization_header.is_empty() {
            tracing::warn!("No Authorization header provided for Pub/Sub verification");
            return Ok(false);
        }

        // Extract token from "Bearer <token>" format
        let parts: Vec<&str> = authorization_header.split_whitespace().collect();
        if parts.len() != 2 || parts[0] != "Bearer" {
            let err = "Invalid Authorization header format (expected 'Bearer <token>')".to_string();
            tracing::warn!("{}", err);
            return Err(AppError::WebhookSignatureVerificationFailed(err).into());
        }

        let token = parts[1];

        // 1. Decode JWT header to extract key ID (kid)
        let header = decode_header(token)
            .map_err(|e| {
                let err = format!("Failed to decode JWT header: {}", e);
                AppError::WebhookSignatureVerificationFailed(err)
            })?;

        let kid = header.kid
            .ok_or_else(|| {
                let err = "JWT missing 'kid' (Key ID) in header".to_string();
                AppError::WebhookSignatureVerificationFailed(err)
            })?;

        // 2. Fetch Google's public keys
        let public_keys = self.get_google_public_keys().await?;

        // 3. Find matching key by kid
        let key = public_keys.iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| {
                let err = format!("No public key found with kid: {}", kid);
                AppError::WebhookSignatureVerificationFailed(err)
            })?;

        // 4. Manually verify JWT and extract claims
        let claims = Self::verify_jwt_with_jwk(token, key, self.skip_rsa)?;

        // 5. Verify audience if configured (security requirement for production)
        if self.verify_aud {
            if self.pub_sub_audience.is_empty() {
                let err = "GOOGLE_VERIFY_AUDIENCE=true but GOOGLE_PUB_SUB_AUDIENCE is not configured".to_string();
                tracing::error!("{}", err);
                return Err(AppError::WebhookSignatureVerificationFailed(err).into());
            }
            
            if claims.aud != self.pub_sub_audience {
                let err = format!(
                    "JWT audience mismatch: got '{}', expected '{}'",
                    claims.aud, self.pub_sub_audience
                );
                tracing::error!("{}", err);
                return Err(AppError::WebhookSignatureVerificationFailed(err).into());
            }
            
            tracing::debug!("JWT audience validated: {}", claims.aud);
        }

        tracing::debug!("Pub/Sub JWT signature verification passed");
        Ok(true)
    }
}

fn order_amount_cents_from_payload(order: &serde_json::Value) -> Option<i32> {
    let total_cents = order["lineItems"]
        .as_array()?
        .iter()
        .filter_map(|line_item| money_cents_from_value(&line_item["total"]))
        .try_fold(0i64, |sum, amount| sum.checked_add(i64::from(amount)))?;

    i32::try_from(total_cents).ok()
}

fn money_cents_from_value(value: &serde_json::Value) -> Option<i32> {
    let units = value["units"].as_str()?.parse::<i64>().ok()?;
    if units < 0 {
        return None;
    }

    let nanos = i64::from(value["nanos"].as_i64().unwrap_or(0) as i32).clamp(0, 999_999_999);
    let cents = units.checked_mul(100)?.checked_add(nanos / 10_000_000)?;
    i32::try_from(cents).ok()
}

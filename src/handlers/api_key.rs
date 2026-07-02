use crate::error::BridgeError;
use crate::ports::traits::{ApiKeyRepository, AppLookupRepository};
use crate::state::AppState;
use axum::{
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
    Extension, Json,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const WEBHOOK_SECRET_PROOF_TTL_SECS: i64 = 5 * 60;

#[derive(Clone)]
pub struct AppAuth {
    pub app_id: Uuid,
    pub api_key_id: Uuid,
    pub is_valid: bool,
}

#[derive(Deserialize)]
pub struct VerifyExpectedAppRequest {
    pub expected_slug: String,
    pub webhook_secret_nonce: String,
    pub webhook_secret_issued_at: i64,
    pub webhook_secret_proof: String,
}

pub async fn verify_expected_app(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<VerifyExpectedAppRequest>,
) -> Result<StatusCode, BridgeError> {
    let expected_slug = payload.expected_slug.trim();
    if expected_slug.is_empty() {
        return Err(BridgeError::BadRequest("expected_slug is required".to_string()));
    }
    let webhook_secret_nonce = payload.webhook_secret_nonce.trim();
    if webhook_secret_nonce.is_empty() {
        return Err(BridgeError::BadRequest("webhook_secret_nonce is required".to_string()));
    }
    let now = unix_timestamp()?;
    let proof_age_secs = now - payload.webhook_secret_issued_at;
    if !(0..=WEBHOOK_SECRET_PROOF_TTL_SECS).contains(&proof_age_secs) {
        return Err(BridgeError::Forbidden(
            "WEBHOOK_CALLBACK_SECRET proof is expired or issued in the future".to_string(),
        ));
    }
    let webhook_secret_proof = payload.webhook_secret_proof.trim();
    if webhook_secret_proof.is_empty() {
        return Err(BridgeError::BadRequest("webhook_secret_proof is required".to_string()));
    }
    let provided_proof = webhook_secret_proof
        .strip_prefix("sha256=")
        .ok_or_else(|| BridgeError::BadRequest("webhook_secret_proof must use sha256= prefix".to_string()))?;
    let provided_proof_bytes = hex::decode(provided_proof)
        .map_err(|_| BridgeError::BadRequest("webhook_secret_proof must be valid hex".to_string()))?;

    let database = state.database();
    let pool = database.pool();

    let summaries = crate::db::apps::list_app_summaries(pool).await?;

    let expected_app_id = summaries
        .iter()
        .find(|s| s.slug == expected_slug)
        .map(|s| s.id);

    let expected_app = if let Some(app_id) = expected_app_id {
        Some(database.as_ref().get_app(app_id).await?)
    } else {
        None
    };

    let api_key_matches_expected_app = auth.is_valid && Some(auth.app_id) == expected_app_id;
    let authenticated_app_id = if auth.is_valid { auth.app_id.to_string() } else { "none".to_string() };
    let authenticated_api_key_id = if auth.is_valid { auth.api_key_id.to_string() } else { "none".to_string() };
    let expected_app_id_for_log = expected_app_id
        .map(|app_id| app_id.to_string())
        .unwrap_or_else(|| "not_found".to_string());

    let mut webhook_secret_matches_expected_app = false;
    if let Some(ref app) = expected_app {
        let mut mac = HmacSha256::new_from_slice(app.webhook_callback_secret.as_bytes())
            .map_err(|_| BridgeError::InternalServerError("HMAC init failed".to_string()))?;
        let signed_message = format!("{}:{}:{}", webhook_secret_nonce, expected_slug, payload.webhook_secret_issued_at);
        mac.update(signed_message.as_bytes());
        webhook_secret_matches_expected_app = mac.verify_slice(&provided_proof_bytes).is_ok();
    }

    if !api_key_matches_expected_app {
        tracing::error!(
            authenticated_app_id = %authenticated_app_id,
            authenticated_api_key_id = %authenticated_api_key_id,
            expected_app_id = %expected_app_id_for_log,
            expected_slug = %expected_slug,
            api_key_valid = %auth.is_valid,
            "Bridge API key app mismatch or invalid key"
        );
    }

    if !webhook_secret_matches_expected_app {
        tracing::error!(
            authenticated_app_id = %authenticated_app_id,
            authenticated_api_key_id = %authenticated_api_key_id,
            expected_app_id = %expected_app_id_for_log,
            expected_slug = %expected_slug,
            "Bridge webhook callback secret mismatch"
        );
    }

    let mut mismatches = Vec::new();
    if !api_key_matches_expected_app {
        mismatches.push("BRIDGE_API_KEY");
    }
    if !webhook_secret_matches_expected_app {
        mismatches.push("WEBHOOK_CALLBACK_SECRET");
    }

    if mismatches.is_empty() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        let vars = mismatches.join(" and ");
        let verb = if mismatches.len() == 1 { "is" } else { "are" };
        Err(BridgeError::Forbidden(
            format!("{} {} not valid for expected Bridge app", vars, verb),
        ))
    }
}

pub async fn api_key_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, BridgeError> {
    if request.method() == Method::OPTIONS {
        return Ok(next.run(request).await);
    }

    let is_verify_path = request.uri().path() == "/api/v1/app/verify" || request.uri().path() == "/app/verify";

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let api_key = match auth_header {
        Some(header) => {
            let parts: Vec<&str> = header.split_whitespace().collect();
            if parts.len() != 2 || parts[0] != "Bearer" {
                if is_verify_path {
                    request.extensions_mut().insert(AppAuth {
                        app_id: Uuid::nil(),
                        api_key_id: Uuid::nil(),
                        is_valid: false,
                    });
                    return Ok(next.run(request).await);
                }

                return Err(BridgeError::UnauthorizedError(
                    "Invalid authorization header format".to_string(),
                ));
            }
            parts[1]
        }
        None => {
            if is_verify_path {
                request.extensions_mut().insert(AppAuth {
                    app_id: Uuid::nil(),
                    api_key_id: Uuid::nil(),
                    is_valid: false,
                });
                return Ok(next.run(request).await);
            }

            return Err(BridgeError::UnauthorizedError("Missing authorization header".to_string()));
        }
    };

    let database = state.database();
    let auth_ctx = match database.as_ref().authenticate_api_key(api_key).await {
        Ok(auth) => {
            let context = AppAuth {
                app_id: auth.app_id,
                api_key_id: auth.api_key_id,
                is_valid: true,
            };
            request.extensions_mut().insert(context.clone());
            Some(context)
        }
        Err(BridgeError::UnauthorizedError(_)) if is_verify_path => {
            let context = AppAuth {
                app_id: Uuid::nil(),
                api_key_id: Uuid::nil(),
                is_valid: false,
            };
            request.extensions_mut().insert(context.clone());
            Some(context)
        }
        Err(e) => return Err(e),
    };

    let mut response = next.run(request).await;
    if let Some(context) = auth_ctx {
        response.extensions_mut().insert(context);
    }
    Ok(response)
}

fn unix_timestamp() -> Result<i64, BridgeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BridgeError::InternalServerError("System clock is before Unix epoch".to_string()))?;
    Ok(duration.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::sync::Arc;
    use sqlx::PgPool;

    async fn test_database() -> Result<Option<crate::db::Database>, Box<dyn Error>> {
        dotenvy::dotenv().ok();
        let admin_database_url = std::env::var("ADMIN_DATABASE_URL").ok();
        let environment = std::env::var("ENVIRONMENT")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_production = matches!(environment.as_str(), "production" | "prod");
        let database_url = match std::env::var("BRIDGE_TEST_DATABASE_URL") {
            Ok(url) => Some(url),
            Err(_) if !is_production => {
                admin_database_url.clone().or_else(|| std::env::var("DATABASE_URL").ok())
            }
            Err(_) => None,
        };
        let Some(database_url) = database_url else {
            return Ok(None);
        };

        Ok(Some(
            crate::db::Database::new(&database_url, admin_database_url.as_deref()).await?,
        ))
    }

    async fn insert_test_app(pool: &PgPool, app_id: Uuid, slug: &str, secret: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO pay.apps (id, slug, display_name, webhook_callback_url, webhook_callback_secret)
             VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(app_id)
        .bind(slug)
        .bind("Verification Test App")
        .bind("http://localhost:1234")
        .bind(secret)
        .execute(pool)
        .await?;

        Ok(())
    }

    async fn delete_test_app(pool: &PgPool, app_id: Uuid) {
        let _ = sqlx::query("DELETE FROM pay.apps WHERE id = $1")
            .bind(app_id)
            .execute(pool)
            .await;
    }

    #[tokio::test]
    async fn test_verify_expected_app_with_invalid_api_key_and_valid_webhook_secret() -> Result<(), Box<dyn Error>> {
        let Some(database) = test_database().await? else {
            eprintln!("skipping test; set BRIDGE_TEST_DATABASE_URL");
            return Ok(());
        };

        let pool = database.pool().clone();
        let app_id = Uuid::new_v4();
        let slug = format!("verify-test-{}", app_id);
        let secret = "my_super_secret_callback_key";
        insert_test_app(&pool, app_id, &slug, secret).await?;

        let state = AppState::new(Arc::new(database));

        let auth = AppAuth {
            app_id: Uuid::nil(),
            api_key_id: Uuid::nil(),
            is_valid: false,
        };

        let nonce = "testnonce123";
        let issued_at = unix_timestamp()?;
        let signed_message = format!("{}:{}:{}", nonce, slug, issued_at);
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
        mac.update(signed_message.as_bytes());
        let proof_bytes = mac.finalize().into_bytes();
        let proof_hex = format!("sha256={}", hex::encode(proof_bytes));

        let payload = VerifyExpectedAppRequest {
            expected_slug: slug.clone(),
            webhook_secret_nonce: nonce.to_string(),
            webhook_secret_issued_at: issued_at,
            webhook_secret_proof: proof_hex,
        };

        let result = verify_expected_app(State(state.clone()), Extension(auth), Json(payload)).await;

        delete_test_app(&pool, app_id).await;

        match result {
            Err(BridgeError::Forbidden(msg)) => {
                assert!(msg.contains("BRIDGE_API_KEY"));
                assert!(!msg.contains("WEBHOOK_CALLBACK_SECRET"));
                assert!(msg.contains("is not valid for expected Bridge app"));
            }
            other => panic!("expected Forbidden error, got {:?}", other),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_verify_expected_app_with_invalid_api_key_and_invalid_webhook_secret() -> Result<(), Box<dyn Error>> {
        let Some(database) = test_database().await? else {
            eprintln!("skipping test; set BRIDGE_TEST_DATABASE_URL");
            return Ok(());
        };

        let pool = database.pool().clone();
        let app_id = Uuid::new_v4();
        let slug = format!("verify-test-{}", app_id);
        let secret = "my_super_secret_callback_key";
        insert_test_app(&pool, app_id, &slug, secret).await?;

        let state = AppState::new(Arc::new(database));

        let auth = AppAuth {
            app_id: Uuid::nil(),
            api_key_id: Uuid::nil(),
            is_valid: false,
        };

        let nonce = "testnonce123";
        let incorrect_proof = "sha256=0000000000000000000000000000000000000000000000000000000000000000";

        let payload = VerifyExpectedAppRequest {
            expected_slug: slug.clone(),
            webhook_secret_nonce: nonce.to_string(),
            webhook_secret_issued_at: unix_timestamp()?,
            webhook_secret_proof: incorrect_proof.to_string(),
        };

        let result = verify_expected_app(State(state.clone()), Extension(auth), Json(payload)).await;

        delete_test_app(&pool, app_id).await;

        match result {
            Err(BridgeError::Forbidden(msg)) => {
                assert!(msg.contains("BRIDGE_API_KEY"));
                assert!(msg.contains("WEBHOOK_CALLBACK_SECRET"));
                assert!(msg.contains("are not valid for expected Bridge app"));
            }
            other => panic!("expected Forbidden error, got {:?}", other),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_verify_expected_app_rejects_expired_webhook_secret_proof() -> Result<(), Box<dyn Error>> {
        let Some(database) = test_database().await? else {
            eprintln!("skipping test; set BRIDGE_TEST_DATABASE_URL");
            return Ok(());
        };

        let pool = database.pool().clone();
        let app_id = Uuid::new_v4();
        let slug = format!("verify-test-{}", app_id);
        let secret = "my_super_secret_callback_key";
        insert_test_app(&pool, app_id, &slug, secret).await?;

        let state = AppState::new(Arc::new(database));
        let auth = AppAuth {
            app_id: Uuid::nil(),
            api_key_id: Uuid::nil(),
            is_valid: false,
        };

        let nonce = "testnonce123";
        let issued_at = unix_timestamp()? - WEBHOOK_SECRET_PROOF_TTL_SECS - 1;
        let signed_message = format!("{}:{}:{}", nonce, slug, issued_at);
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
        mac.update(signed_message.as_bytes());
        let proof_hex = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let payload = VerifyExpectedAppRequest {
            expected_slug: slug.clone(),
            webhook_secret_nonce: nonce.to_string(),
            webhook_secret_issued_at: issued_at,
            webhook_secret_proof: proof_hex,
        };

        let result = verify_expected_app(State(state.clone()), Extension(auth), Json(payload)).await;

        delete_test_app(&pool, app_id).await;

        match result {
            Err(BridgeError::Forbidden(msg)) => {
                assert!(msg.contains("expired or issued in the future"));
            }
            other => panic!("expected expired Forbidden error, got {:?}", other),
        }

        Ok(())
    }
}

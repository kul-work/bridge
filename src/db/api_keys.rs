use crate::error::BridgeError;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use bcrypt::verify as bcrypt_verify;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;

/// API key for app authentication
/// Used by api_key middleware for authentication. Struct construction is handled by SQLx FromRow.
#[allow(dead_code)]
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub app_id: Uuid,
    pub key_prefix: String,
    pub key_hash: String,
    pub label: Option<String>,
    pub permissions: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, FromRow)]
struct ApiKeyCandidate {
    id: Uuid,
    app_id: Uuid,
    key_hash: String,
    app_enabled: bool,
}

pub async fn authenticate_api_key(pool: &PgPool, raw_key: &str) -> Result<Uuid, BridgeError> {
    let key_prefix: String = raw_key.chars().take(8).collect();
    if key_prefix.len() < 8 {
        return Err(BridgeError::UnauthorizedError("Invalid API key".to_string()));
    }

    let candidates = sqlx::query_as::<_, ApiKeyCandidate>(
        "SELECT k.id, k.app_id, k.key_hash, a.enabled AS app_enabled
         FROM pay.api_keys k
         JOIN pay.apps a ON a.id = k.app_id
         WHERE k.enabled = true
           AND k.key_prefix = $1",
    )
    .bind(&key_prefix)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    for candidate in candidates {
        if verify_api_key(raw_key, &candidate.key_hash) {
            if !candidate.app_enabled {
                return Err(BridgeError::AppDisabled("Application is disabled".to_string()));
            }

            sqlx::query(
                "UPDATE pay.api_keys SET last_used_at = NOW() WHERE id = $1",
            )
            .bind(candidate.id)
            .execute(pool)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

            return Ok(candidate.app_id);
        }
    }

    Err(BridgeError::UnauthorizedError("Invalid API key".to_string()))
}

fn verify_api_key(raw_key: &str, stored_hash: &str) -> bool {
    if stored_hash.starts_with("$2a$")
        || stored_hash.starts_with("$2b$")
        || stored_hash.starts_with("$2x$")
        || stored_hash.starts_with("$2y$")
    {
        return bcrypt_verify(raw_key, stored_hash).unwrap_or(false);
    }

    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(raw_key.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

use crate::error::BridgeError;
use crate::db::database::set_local_app_id;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use bcrypt::verify as bcrypt_verify;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct AuthenticatedApiKey {
    pub api_key_id: Uuid,
    pub app_id: Uuid,
}

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

pub async fn authenticate_api_key(
    pool: &PgPool,
    raw_key: &str,
) -> Result<AuthenticatedApiKey, BridgeError> {
    let key_prefix: String = raw_key.chars().take(8).collect();
    if key_prefix.len() < 8 {
        return Err(BridgeError::UnauthorizedError("Invalid API key".to_string()));
    }

    let candidates = sqlx::query_as::<_, ApiKeyCandidate>(
        "SELECT id, app_id, key_hash, app_enabled
         FROM pay.get_api_key_auth_candidates_bootstrap($1)",
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

            let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            set_local_app_id(&mut tx, candidate.app_id).await?;

            sqlx::query(
                "UPDATE pay.api_keys SET last_used_at = NOW() WHERE id = $1",
            )
            .bind(candidate.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

            tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

            return Ok(AuthenticatedApiKey {
                api_key_id: candidate.id,
                app_id: candidate.app_id,
            });
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
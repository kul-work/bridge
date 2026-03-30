use crate::error::BridgeError;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CachedCheckout {
    pub request_fingerprint: String,
    pub response_payload: Value,
}

pub async fn get_cached_checkout(
    pool: &PgPool,
    app_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<CachedCheckout>, BridgeError> {
    let row = sqlx::query_as::<_, (String, Value)>(
        "SELECT request_fingerprint, response_payload
         FROM pay.checkout_idempotency
         WHERE app_id = $1 AND idempotency_key = $2
         LIMIT 1",
    )
    .bind(app_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|(request_fingerprint, response_payload)| CachedCheckout {
        request_fingerprint,
        response_payload,
    }))
}

pub async fn cache_checkout_response(
    pool: &PgPool,
    app_id: Uuid,
    idempotency_key: &str,
    request_fingerprint: &str,
    response_payload: &Value,
) -> Result<(), BridgeError> {
    sqlx::query(
        "INSERT INTO pay.checkout_idempotency (app_id, idempotency_key, request_fingerprint, response_payload)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (app_id, idempotency_key) DO NOTHING",
    )
    .bind(app_id)
    .bind(idempotency_key)
    .bind(request_fingerprint)
    .bind(response_payload)
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

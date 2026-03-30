use crate::error::BridgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub app_id: Uuid,
    pub external_user_id: String,
    pub subscription_id: String,
    pub provider: String,
    pub purchase_token: Option<String>,
    pub status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub cancel_reason: Option<i32>,
    pub provider_customer_id: Option<String>,
    pub version: i32,
    pub last_event_time: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    
    // Google Play specific
    #[serde(default)]
    pub google_requires_price_step_up_consent: Option<bool>,
    #[serde(default)]
    pub google_price_step_up_consent_deadline: Option<DateTime<Utc>>,
    #[serde(default)]
    pub google_new_price_cents: Option<i32>,
    #[serde(default)]
    pub google_pause_scheduled_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub google_paused_at: Option<DateTime<Utc>>,
}

pub async fn get_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
) -> Result<Subscription, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND external_user_id = $2 AND subscription_id = $3 AND provider = $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))
}

pub async fn get_user_subscriptions_keyset(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    cursor_created_at: Option<DateTime<Utc>>,
    cursor_id: Option<Uuid>,
) -> Result<Vec<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions
         WHERE app_id = $1 AND external_user_id = $2 
           AND (
               $3::timestamptz IS NULL
               OR $4::uuid IS NULL
               OR (created_at, id) < ($3, $4)
           )
         ORDER BY created_at DESC, id DESC
         LIMIT $5"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(cursor_created_at)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn get_user_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions
         WHERE app_id = $1 AND external_user_id = $2
         ORDER BY created_at DESC
         LIMIT $3 OFFSET $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn create_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
    status: &str,
) -> Result<Subscription, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "INSERT INTO pay.subscriptions 
        (app_id, external_user_id, subscription_id, provider, status) 
        VALUES ($1, $2, $3, $4, $5) 
        RETURNING *"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .bind(status)
    .fetch_one(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn upsert_subscription_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
    status: &str,
    current_period_end: Option<DateTime<Utc>>,
    purchase_token: Option<&str>,
    auto_renewing: Option<bool>,
    payment_state: Option<i32>,
    provider_customer_id: Option<&str>,
    event_time_ms: i64,
) -> Result<Subscription, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "INSERT INTO pay.subscriptions
         (app_id, external_user_id, subscription_id, provider, status, current_period_end, purchase_token, auto_renewing, payment_state, provider_customer_id, version, last_event_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1, $11)
         ON CONFLICT (app_id, external_user_id, subscription_id, provider)
         DO UPDATE SET
           status = EXCLUDED.status,
           current_period_end = EXCLUDED.current_period_end,
           purchase_token = COALESCE(EXCLUDED.purchase_token, subscriptions.purchase_token),
           auto_renewing = EXCLUDED.auto_renewing,
           payment_state = EXCLUDED.payment_state,
           provider_customer_id = EXCLUDED.provider_customer_id,
           version = subscriptions.version + 1,
           last_event_time = EXCLUDED.last_event_time,
           updated_at = NOW()
         RETURNING *"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .bind(status)
    .bind(current_period_end)
    .bind(purchase_token)
    .bind(auto_renewing)
    .bind(payment_state)
    .bind(provider_customer_id)
    .bind(event_time_ms)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn upsert_pending_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
) -> Result<Subscription, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "INSERT INTO pay.subscriptions
         (app_id, external_user_id, subscription_id, provider, status, version, last_event_time)
         VALUES ($1, $2, $3, $4, 'pending', 1, 0)
         ON CONFLICT (app_id, external_user_id, subscription_id, provider)
         DO UPDATE SET updated_at = NOW()
         RETURNING *"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .fetch_one(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn lookup_user_by_subscription_id(
    pool: &PgPool,
    app_id: Uuid,
    subscription_id: &str,
) -> Result<Option<String>, BridgeError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.subscriptions WHERE app_id = $1 AND subscription_id = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(subscription_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn lookup_user_by_purchase_token(
    pool: &PgPool,
    app_id: Uuid,
    purchase_token: &str,
) -> Result<Option<String>, BridgeError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.subscriptions WHERE app_id = $1 AND purchase_token = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(purchase_token)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn get_subscription_by_sub_id(
    pool: &PgPool,
    app_id: Uuid,
    subscription_id: &str,
) -> Result<Option<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND subscription_id = $2"
    )
    .bind(app_id)
    .bind(subscription_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn get_subscription_by_purchase_token(
    pool: &PgPool,
    app_id: Uuid,
    purchase_token: &str,
) -> Result<Option<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND purchase_token = $2"
    )
    .bind(app_id)
    .bind(purchase_token)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn update_subscription_status(
    pool: &PgPool,
    app_id: Uuid,
    subscription_id: &str,
    new_status: &str,
    event_time_ms: i64,
) -> Result<bool, BridgeError> {
    let result = sqlx::query(
        "UPDATE pay.subscriptions
         SET status = $1, version = version + 1, last_event_time = $2, updated_at = NOW()
         WHERE app_id = $3 AND subscription_id = $4 AND last_event_time < $2"
    )
    .bind(new_status)
    .bind(event_time_ms)
    .bind(app_id)
    .bind(subscription_id)
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() > 0)
}

pub async fn lookup_user_by_google_obfuscated_id(
    pool: &PgPool,
    app_id: Uuid,
    obfuscated_id: &str,
) -> Result<Option<String>, BridgeError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.subscriptions WHERE app_id = $1 AND google_obfuscated_account_id = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(obfuscated_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn link_replacement_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    current_subscription_id: &str,
) -> Result<(), BridgeError> {
    sqlx::query(
        "UPDATE pay.subscriptions 
         SET status = 'replaced', updated_at = NOW() 
         WHERE app_id = $1 
           AND external_user_id = $2 
           AND subscription_id != $3 
           AND status IN ('active', 'trial', 'past_due', 'on_hold')
           AND provider = 'google_play'"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(current_subscription_id)
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

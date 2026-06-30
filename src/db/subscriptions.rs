use crate::db::database::set_local_app_id;
use crate::error::BridgeError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder};
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

    // Cancellation / Revocation
    pub cancellation_initiated_at: Option<DateTime<Utc>>,
    pub revocation_reason: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub payment_failure_notification: bool,

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
    #[serde(default)]
    pub google_deferred_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub google_pending_price_change_new_price_cents: Option<i64>,
    #[serde(default)]
    pub google_pending_price_change_currency: Option<String>,
    #[serde(default)]
    pub google_pending_price_change_mode: Option<String>,
    #[serde(default)]
    pub google_pending_price_change_state: Option<String>,
    #[serde(default)]
    pub google_pending_price_change_expected_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scheduled_job_claim_token: Option<Uuid>,
    #[serde(default)]
    pub scheduled_job_claimed_by: Option<String>,
    #[serde(default)]
    pub scheduled_job_claimed_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scheduled_job_claim_kind: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionUpsertResult {
    pub subscription: Subscription,
    pub applied: bool,
}

async fn begin_app_tx<'a>(
    pool: &'a PgPool,
    app_id: Uuid,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, BridgeError> {
    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;
    Ok(tx)
}

pub use crate::ports::SubscriptionWebhookTransition;

pub async fn apply_webhook_transition(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    subscription_id: &str,
    event_time_ms: i64,
    transition: SubscriptionWebhookTransition,
) -> Result<Option<Subscription>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let result = apply_webhook_transition_tx(
        &mut tx,
        app_id,
        external_user_id,
        provider,
        subscription_id,
        event_time_ms,
        transition,
    )
    .await?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result)
}

pub async fn apply_webhook_transition_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    subscription_id: &str,
    event_time_ms: i64,
    transition: SubscriptionWebhookTransition,
) -> Result<Option<Subscription>, BridgeError> {
    let result = match transition {
        SubscriptionWebhookTransition::Pending => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'pending',
                     google_subscription_state = 5,
                     version = version + 1,
                     last_event_time = $1,
                     updated_at = NOW()
                 WHERE app_id = $2 AND external_user_id = $3 AND provider = $4 AND subscription_id = $5 AND last_event_time < $1
                 RETURNING *",
            )
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::GracePeriod {
            grace_period_end,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'past_due',
                     google_grace_period_start = COALESCE(google_grace_period_start, NOW()),
                     google_grace_period_end = COALESCE($1, google_grace_period_end),
                     google_subscription_state = 2,
                     version = version + 1,
                     last_event_time = $2,
                     updated_at = NOW()
                 WHERE app_id = $3 AND external_user_id = $4 AND provider = $5 AND subscription_id = $6 AND last_event_time < $2
                 RETURNING *",
            )
            .bind(grace_period_end)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::Revoked {
            revocation_reason,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'revoked',
                     auto_renewing = false,
                     revoked_at = NOW(),
                     revocation_reason = COALESCE($1, revocation_reason),
                     google_subscription_state = 6,
                     google_pending_price_change_new_price_cents = NULL,
                     google_pending_price_change_currency = NULL,
                     google_pending_price_change_mode = NULL,
                     google_pending_price_change_state = NULL,
                     google_pending_price_change_expected_at = NULL,
                     version = version + 1,
                     last_event_time = $2,
                     updated_at = NOW()
                 WHERE app_id = $3 AND external_user_id = $4 AND provider = $5 AND subscription_id = $6 AND last_event_time < $2
                 RETURNING *",
            )
            .bind(revocation_reason)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::OnHold => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'on_hold',
                     payment_failure_notification = true,
                     google_subscription_state = 3,
                     version = version + 1,
                     last_event_time = $1,
                     updated_at = NOW()
                 WHERE app_id = $2 AND external_user_id = $3 AND provider = $4 AND subscription_id = $5 AND last_event_time < $1
                 RETURNING *",
            )
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::Paused => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'paused',
                     auto_renewing = false,
                     google_paused_at = NOW(),
                     google_subscription_state = 4,
                     version = version + 1,
                     last_event_time = $1,
                     updated_at = NOW()
                 WHERE app_id = $2 AND external_user_id = $3 AND provider = $4 AND subscription_id = $5 AND last_event_time < $1
                 RETURNING *",
            )
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::Resumed {
            current_period_end,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'active',
                     auto_renewing = true,
                     current_period_end = COALESCE($1, current_period_end),
                     google_paused_at = NULL,
                     google_pause_scheduled_at = NULL,
                     cancellation_initiated_at = NULL,
                     google_subscription_state = 0,
                     version = version + 1,
                     last_event_time = $2,
                     updated_at = NOW()
                 WHERE app_id = $3 AND external_user_id = $4 AND provider = $5 AND subscription_id = $6 AND last_event_time < $2
                 RETURNING *",
            )
            .bind(current_period_end)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::CancellationScheduled {
            google_cancellation_context,
            google_cancellation_feedback,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET auto_renewing = false,
                     cancellation_initiated_at = COALESCE(cancellation_initiated_at, NOW()),
                     google_pending_cancellation = true,
                     google_pending_cancellation_at = COALESCE(google_pending_cancellation_at, NOW()),
                     google_cancellation_context = COALESCE($1, google_cancellation_context),
                     google_cancellation_feedback = COALESCE($2, google_cancellation_feedback),
                     version = version + 1,
                     last_event_time = $3,
                     updated_at = NOW()
                 WHERE app_id = $4 AND external_user_id = $5 AND provider = $6 AND subscription_id = $7 AND last_event_time < $3
                 RETURNING *",
            )
            .bind(google_cancellation_context)
            .bind(google_cancellation_feedback)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::Expired => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'expired',
                     auto_renewing = false,
                     google_subscription_state = 6,
                     google_pending_price_change_new_price_cents = NULL,
                     google_pending_price_change_currency = NULL,
                     google_pending_price_change_mode = NULL,
                     google_pending_price_change_state = NULL,
                     google_pending_price_change_expected_at = NULL,
                     version = version + 1,
                     last_event_time = $1,
                     updated_at = NOW()
                 WHERE app_id = $2 AND external_user_id = $3 AND provider = $4 AND subscription_id = $5 AND last_event_time < $1
                 RETURNING *",
            )
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::Cancelled {
            current_period_end,
            google_cancellation_context,
            google_cancellation_feedback,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'cancelled',
                     auto_renewing = false,
                     current_period_end = COALESCE($1, current_period_end),
                     cancellation_initiated_at = COALESCE(cancellation_initiated_at, NOW()),
                     google_subscription_state = 1,
                     google_cancellation_context = COALESCE($2, google_cancellation_context),
                     google_cancellation_feedback = COALESCE($3, google_cancellation_feedback),
                     google_pending_price_change_new_price_cents = NULL,
                     google_pending_price_change_currency = NULL,
                     google_pending_price_change_mode = NULL,
                     google_pending_price_change_state = NULL,
                     google_pending_price_change_expected_at = NULL,
                     version = version + 1,
                     last_event_time = $4,
                     updated_at = NOW()
                 WHERE app_id = $5 AND external_user_id = $6 AND provider = $7 AND subscription_id = $8 AND last_event_time < $4
                 RETURNING *",
            )
            .bind(current_period_end)
            .bind(google_cancellation_context)
            .bind(google_cancellation_feedback)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::PaymentFailed => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET payment_failure_notification = true,
                     version = version + 1,
                     last_event_time = CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END,
                     updated_at = NOW()
                 WHERE app_id = $2 AND external_user_id = $3 AND provider = $4 AND subscription_id = $5
                 RETURNING *",
            )
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::PendingPurchaseCancelled => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'cancelled',
                     auto_renewing = false,
                     cancellation_initiated_at = COALESCE(cancellation_initiated_at, NOW()),
                     revocation_reason = 'pending_purchase_canceled',
                     google_subscription_state = 1,
                     google_pending_price_change_new_price_cents = NULL,
                     google_pending_price_change_currency = NULL,
                     google_pending_price_change_mode = NULL,
                     google_pending_price_change_state = NULL,
                     google_pending_price_change_expected_at = NULL,
                     version = version + 1,
                     last_event_time = $1,
                     updated_at = NOW()
                 WHERE app_id = $2 AND external_user_id = $3 AND provider = $4 AND subscription_id = $5 AND last_event_time < $1
                 RETURNING *",
            )
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::PriceStepUp {
            google_new_price_cents,
            google_price_step_up_consent_deadline,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET google_requires_price_step_up_consent = true,
                     google_new_price_cents = $1,
                     google_price_step_up_consent_deadline = COALESCE($2, google_price_step_up_consent_deadline),
                     version = version + 1,
                     last_event_time = $3,
                     updated_at = NOW()
                 WHERE app_id = $4 AND external_user_id = $5 AND provider = $6 AND subscription_id = $7 AND last_event_time < $3
                 RETURNING *",
            )
            .bind(google_new_price_cents)
            .bind(google_price_step_up_consent_deadline)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::PendingPriceChange {
            new_price_cents,
            currency,
            mode,
            state,
            expected_at,
        } => {
            let clears_pending = matches!(state.as_deref(), Some("APPLIED") | Some("CANCELED"));
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET google_pending_price_change_new_price_cents = CASE WHEN $1 THEN NULL ELSE $2 END,
                     google_pending_price_change_currency = CASE WHEN $1 THEN NULL ELSE $3 END,
                     google_pending_price_change_mode = CASE WHEN $1 THEN NULL ELSE $4 END,
                     google_pending_price_change_state = $5,
                     google_pending_price_change_expected_at = CASE WHEN $1 THEN NULL ELSE $6 END,
                     version = version + 1,
                     last_event_time = $7,
                     updated_at = NOW()
                 WHERE app_id = $8 AND external_user_id = $9 AND provider = $10 AND subscription_id = $11 AND last_event_time < $7
                 RETURNING *",
            )
            .bind(clears_pending)
            .bind(new_price_cents)
            .bind(currency)
            .bind(mode)
            .bind(state)
            .bind(expected_at)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::PauseScheduled {
            google_pause_scheduled_at,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET google_pause_scheduled_at = $1,
                     version = version + 1,
                     last_event_time = $2,
                     updated_at = NOW()
                 WHERE app_id = $3 AND external_user_id = $4 AND provider = $5 AND subscription_id = $6 AND last_event_time < $2
                 RETURNING *",
            )
            .bind(google_pause_scheduled_at)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
        SubscriptionWebhookTransition::Deferred {
            google_deferred_until,
        } => {
            sqlx::query_as::<_, Subscription>(
                "UPDATE pay.subscriptions
                 SET google_deferred_until = $1,
                     version = version + 1,
                     last_event_time = $2,
                     updated_at = NOW()
                 WHERE app_id = $3 AND external_user_id = $4 AND provider = $5 AND subscription_id = $6 AND last_event_time < $2
                 RETURNING *",
            )
            .bind(google_deferred_until)
            .bind(event_time_ms)
            .bind(app_id)
            .bind(external_user_id)
            .bind(provider)
            .bind(subscription_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
    };

    Ok(result)
}

pub async fn get_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND external_user_id = $2 AND subscription_id = $3 AND provider = $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    subscription.ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))
}

pub async fn get_user_subscriptions_keyset(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    cursor_created_at: Option<DateTime<Utc>>,
    cursor_id: Option<Uuid>,
) -> Result<Vec<Subscription>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscriptions = sqlx::query_as::<_, Subscription>(
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
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscriptions)
}

pub async fn get_user_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Subscription>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscriptions = sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions
         WHERE app_id = $1 AND external_user_id = $2
         ORDER BY created_at DESC
         LIMIT $3 OFFSET $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscriptions)
}

pub async fn has_live_subscription_for_product(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    product_id: &str,
) -> Result<bool, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1
             FROM pay.subscriptions s
             JOIN pay.payments p
               ON p.app_id = s.app_id
              AND p.external_user_id = s.external_user_id
              AND p.provider = s.provider
              AND p.subscription_id = s.subscription_id
             WHERE s.app_id = $1
               AND s.external_user_id = $2
               AND s.provider = $3
               AND p.product_id = $4
               AND lower(s.status) IN ('active', 'trial', 'trialing')
         )"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(provider)
    .bind(product_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(exists)
}

pub async fn list_reconciliation_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
) -> Result<Vec<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions
         WHERE app_id = $1 AND status IN ('active', 'trial', 'past_due')"
    )
    .bind(app_id)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn claim_price_step_up_expired_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
    worker_id: &str,
    lease_secs: i64,
    limit: i64,
) -> Result<Vec<Subscription>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let subscriptions = sqlx::query_as::<_, Subscription>(
        "WITH candidates AS (
             SELECT id
             FROM pay.subscriptions
             WHERE app_id = $1
               AND google_requires_price_step_up_consent = true
               AND google_price_step_up_consent_deadline IS NOT NULL
               AND google_price_step_up_consent_deadline < NOW()
               AND (
                   scheduled_job_claimed_until IS NULL
                   OR scheduled_job_claimed_until < NOW()
               )
             ORDER BY google_price_step_up_consent_deadline ASC
             LIMIT $4
             FOR UPDATE SKIP LOCKED
         )
         UPDATE pay.subscriptions s
         SET scheduled_job_claim_token = gen_random_uuid(),
             scheduled_job_claimed_by = $2,
             scheduled_job_claimed_until = NOW() + ($3 * INTERVAL '1 second'),
             scheduled_job_claim_kind = 'price_step_up_expiry',
             updated_at = NOW()
         FROM candidates
         WHERE s.id = candidates.id AND s.app_id = $1
         RETURNING s.*"
    )
    .bind(app_id)
    .bind(worker_id)
    .bind(lease_secs)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscriptions)
}

pub async fn list_pending_pause_subscriptions(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions
         WHERE google_pause_scheduled_at IS NOT NULL
           AND google_pause_scheduled_at <= NOW()
           AND status != 'paused'
         LIMIT $1"
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn mark_subscription_price_step_up_expired(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
    claim_token: Uuid,
    event_time_ms: i64,
) -> Result<bool, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let result = sqlx::query(
        "UPDATE pay.subscriptions
         SET google_requires_price_step_up_consent = false,
             google_price_step_up_consent_deadline = NULL,
             status = 'cancelled',
             revocation_reason = 'price_step_up_expiry',
             auto_renewing = false,
             scheduled_job_claim_token = NULL,
             scheduled_job_claimed_by = NULL,
             scheduled_job_claimed_until = NULL,
             scheduled_job_claim_kind = NULL,
             version = version + 1,
             last_event_time = CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END,
             updated_at = NOW()
         WHERE app_id = $2
           AND id = $3
           AND scheduled_job_claim_token = $4
           AND scheduled_job_claim_kind = 'price_step_up_expiry'
           AND google_requires_price_step_up_consent = true
           AND google_price_step_up_consent_deadline IS NOT NULL
           AND google_price_step_up_consent_deadline < NOW()"
    )
    .bind(event_time_ms)
    .bind(app_id)
    .bind(id)
    .bind(claim_token)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() > 0)
}

pub async fn refresh_subscription_scheduler_claim(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
    claim_token: Uuid,
    claim_kind: &str,
    lease_secs: i64,
) -> Result<bool, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let result = sqlx::query(
        "UPDATE pay.subscriptions
         SET scheduled_job_claimed_until = NOW() + ($5 * INTERVAL '1 second'),
             updated_at = NOW()
         WHERE app_id = $1
           AND id = $2
           AND scheduled_job_claim_token = $3
           AND scheduled_job_claim_kind = $4
           AND google_requires_price_step_up_consent = true
           AND google_price_step_up_consent_deadline IS NOT NULL
           AND google_price_step_up_consent_deadline < NOW()"
    )
    .bind(app_id)
    .bind(id)
    .bind(claim_token)
    .bind(claim_kind)
    .bind(lease_secs)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() == 1)
}

pub async fn mark_subscription_paused(
    pool: &PgPool,
    id: Uuid,
    event_time_ms: i64,
) -> Result<bool, BridgeError> {
    let result = sqlx::query(
        "UPDATE pay.subscriptions
         SET status = 'paused',
             auto_renewing = false,
             google_paused_at = NOW(),
             version = version + 1,
             last_event_time = CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END,
             updated_at = NOW()
         WHERE id = $2 AND status != 'paused'"
    )
    .bind(event_time_ms)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() > 0)
}

pub async fn delete_orphaned_pending_subscriptions(pool: &PgPool) -> Result<u64, BridgeError> {
    let result = sqlx::query(
        "DELETE FROM pay.subscriptions
         WHERE status = 'pending'
           AND purchase_token IS NULL
           AND created_at < NOW() - INTERVAL '30 minutes'"
    )
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected())
}

#[allow(clippy::too_many_arguments)]
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
    recurring_amount_cents: Option<i64>,
) -> Result<SubscriptionUpsertResult, BridgeError> {
    // If purchase_token is provided, check if a subscription with that token already exists
    // This handles Google Play renewals where the same purchase_token is used for the lifecycle
    if let Some(token) = purchase_token {
        let existing = sqlx::query_as::<_, Subscription>(
            "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND purchase_token = $2"
        )
        .bind(app_id)
        .bind(token)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

        if let Some(existing_sub) = existing {
            // Update existing subscription found by purchase_token
            // Only apply if the new event is newer or equal (stale event suppression)
            if existing_sub.last_event_time <= event_time_ms {
                let updated = sqlx::query_as::<_, Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = $1, current_period_end = COALESCE($2, current_period_end),
                         auto_renewing = COALESCE($3, auto_renewing),
                         payment_state = COALESCE($4, payment_state),
                         provider_customer_id = COALESCE($5, provider_customer_id),
                         google_grace_period_start = CASE WHEN $1 = 'active' THEN NULL ELSE google_grace_period_start END,
                         google_grace_period_end = CASE WHEN $1 = 'active' THEN NULL ELSE google_grace_period_end END,
                         payment_failure_notification = CASE WHEN $1 = 'active' THEN false ELSE payment_failure_notification END,
                         google_pending_price_change_new_price_cents = CASE WHEN $1 IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $8::bigint IS NOT NULL AND google_pending_price_change_new_price_cents = $8 THEN NULL ELSE google_pending_price_change_new_price_cents END,
                         google_pending_price_change_currency = CASE WHEN $1 IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $8::bigint IS NOT NULL AND google_pending_price_change_new_price_cents = $8 THEN NULL ELSE google_pending_price_change_currency END,
                         google_pending_price_change_mode = CASE WHEN $1 IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $8::bigint IS NOT NULL AND google_pending_price_change_new_price_cents = $8 THEN NULL ELSE google_pending_price_change_mode END,
                         google_pending_price_change_state = CASE WHEN $1 IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $8::bigint IS NOT NULL AND google_pending_price_change_new_price_cents = $8 THEN NULL ELSE google_pending_price_change_state END,
                         google_pending_price_change_expected_at = CASE WHEN $1 IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $8::bigint IS NOT NULL AND google_pending_price_change_new_price_cents = $8 THEN NULL ELSE google_pending_price_change_expected_at END,
                         version = version + 1, last_event_time = $6, updated_at = NOW()
                     WHERE id = $7
                     RETURNING *"
                )
                .bind(status)
                .bind(current_period_end)
                .bind(auto_renewing)
                .bind(payment_state)
                .bind(provider_customer_id)
                .bind(event_time_ms)
                .bind(existing_sub.id)
                .bind(recurring_amount_cents)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;

                return Ok(SubscriptionUpsertResult {
                    subscription: updated,
                    applied: true,
                });
            } else {
                // Stale event - return existing without applying
                return Ok(SubscriptionUpsertResult {
                    subscription: existing_sub,
                    applied: false,
                });
            }
        }
    }

    let subscription = sqlx::query_as::<_, Subscription>(
        "INSERT INTO pay.subscriptions AS subscriptions
         (app_id, external_user_id, subscription_id, provider, status, current_period_end, purchase_token, auto_renewing, payment_state, provider_customer_id, version, last_event_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1, $11)
         ON CONFLICT (app_id, external_user_id, subscription_id, provider)
         DO UPDATE SET
           status = EXCLUDED.status,
           current_period_end = COALESCE(EXCLUDED.current_period_end, subscriptions.current_period_end),
           purchase_token = COALESCE(EXCLUDED.purchase_token, subscriptions.purchase_token),
           auto_renewing = COALESCE(EXCLUDED.auto_renewing, subscriptions.auto_renewing),
           payment_state = COALESCE(EXCLUDED.payment_state, subscriptions.payment_state),
           provider_customer_id = COALESCE(EXCLUDED.provider_customer_id, subscriptions.provider_customer_id),
           google_grace_period_start = CASE WHEN EXCLUDED.status = 'active' THEN NULL ELSE subscriptions.google_grace_period_start END,
           google_grace_period_end = CASE WHEN EXCLUDED.status = 'active' THEN NULL ELSE subscriptions.google_grace_period_end END,
           payment_failure_notification = CASE WHEN EXCLUDED.status = 'active' THEN false ELSE subscriptions.payment_failure_notification END,
           google_pending_price_change_new_price_cents = CASE WHEN EXCLUDED.status IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $12::bigint IS NOT NULL AND subscriptions.google_pending_price_change_new_price_cents = $12 THEN NULL ELSE subscriptions.google_pending_price_change_new_price_cents END,
           google_pending_price_change_currency = CASE WHEN EXCLUDED.status IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $12::bigint IS NOT NULL AND subscriptions.google_pending_price_change_new_price_cents = $12 THEN NULL ELSE subscriptions.google_pending_price_change_currency END,
           google_pending_price_change_mode = CASE WHEN EXCLUDED.status IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $12::bigint IS NOT NULL AND subscriptions.google_pending_price_change_new_price_cents = $12 THEN NULL ELSE subscriptions.google_pending_price_change_mode END,
           google_pending_price_change_state = CASE WHEN EXCLUDED.status IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $12::bigint IS NOT NULL AND subscriptions.google_pending_price_change_new_price_cents = $12 THEN NULL ELSE subscriptions.google_pending_price_change_state END,
           google_pending_price_change_expected_at = CASE WHEN EXCLUDED.status IN ('cancelled', 'expired', 'revoked') THEN NULL WHEN $12::bigint IS NOT NULL AND subscriptions.google_pending_price_change_new_price_cents = $12 THEN NULL ELSE subscriptions.google_pending_price_change_expected_at END,
           version = subscriptions.version + 1,
           last_event_time = EXCLUDED.last_event_time,
           updated_at = NOW()
         WHERE subscriptions.last_event_time < EXCLUDED.last_event_time
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
    .bind(recurring_amount_cents)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    if let Some(subscription) = subscription {
        return Ok(SubscriptionUpsertResult {
            subscription,
            applied: true,
        });
    }

    let subscription = sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND external_user_id = $2 AND subscription_id = $3 AND provider = $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(SubscriptionUpsertResult {
        subscription,
        applied: false,
    })
}

pub async fn upsert_pending_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "INSERT INTO pay.subscriptions
         (app_id, external_user_id, subscription_id, provider, status, version, last_event_time)
         VALUES ($1, $2, $3, $4, 'pending', 1, 0)
         ON CONFLICT (app_id, external_user_id, subscription_id, provider)
         DO UPDATE SET
           updated_at = NOW()
         WHERE subscriptions.last_event_time <= EXCLUDED.last_event_time
         RETURNING *"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let subscription = match subscription {
        Some(sub) => sub,
        None => {
            sqlx::query_as::<_, Subscription>(
                "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND external_user_id = $2 AND subscription_id = $3 AND provider = $4"
            )
            .bind(app_id)
            .bind(external_user_id)
            .bind(subscription_id)
            .bind(provider)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?
        }
    };

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

/// Remove a pending subscription placeholder created by /register when
/// verification returns linking_required (purchase belongs to a different account).
pub async fn delete_pending_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
) -> Result<(), BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    sqlx::query(
        "DELETE FROM pay.subscriptions
         WHERE app_id = $1 AND external_user_id = $2 AND subscription_id = $3
           AND provider = $4 AND status = 'pending'"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn cancel_subscription_scheduled(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "UPDATE pay.subscriptions
         SET auto_renewing = false, cancellation_initiated_at = NOW(), updated_at = NOW()
         WHERE app_id = $1 AND id = $2
         RETURNING *",
    )
    .bind(app_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

pub async fn cancel_subscription_immediate(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "UPDATE pay.subscriptions
         SET status = 'cancelled',
             auto_renewing = false,
             cancellation_initiated_at = NOW(),
             current_period_end = NOW(),
             revocation_reason = 'immediate_cancel',
             revoked_at = NOW(),
             updated_at = NOW()
         WHERE app_id = $1 AND id = $2
         RETURNING *",
    )
    .bind(app_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

pub async fn resume_subscription(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "UPDATE pay.subscriptions
         SET status = 'active', auto_renewing = true, cancellation_initiated_at = NULL, updated_at = NOW()
         WHERE app_id = $1 AND id = $2
         RETURNING *",
    )
    .bind(app_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

pub async fn mark_payment_acknowledged_for_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    subscription_id: &str,
    purchase_token: Option<&str>,
) -> Result<(), BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    if let Some(purchase_token) = purchase_token {
        sqlx::query(
            "UPDATE pay.payments
             SET acknowledged_at = COALESCE(acknowledged_at, NOW())
             WHERE app_id = $1
               AND external_user_id = $2
               AND provider = $3
               AND (
                 provider_transaction_id = $4
                 OR (provider_purchase_token = $4 AND ack_required = true)
               )",
        )
        .bind(app_id)
        .bind(external_user_id)
        .bind(provider)
        .bind(purchase_token)
        .execute(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;
    } else {
        sqlx::query(
            "UPDATE pay.payments
             SET acknowledged_at = COALESCE(acknowledged_at, NOW())
             WHERE app_id = $1 AND external_user_id = $2 AND provider = $3 AND subscription_id = $4",
        )
        .bind(app_id)
        .bind(external_user_id)
        .bind(provider)
        .bind(subscription_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;
    }

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn clear_payment_failure_notification(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    subscription_id: &str,
) -> Result<(), BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    sqlx::query(
        "UPDATE pay.subscriptions
         SET payment_failure_notification = false
         WHERE app_id = $1 AND external_user_id = $2 AND provider = $3 AND subscription_id = $4",
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(provider)
    .bind(subscription_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn accept_price_step_up(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "UPDATE pay.subscriptions
         SET google_requires_price_step_up_consent = false,
             google_price_step_up_consent_status = 'accepted',
             google_price_step_up_consent_deadline = NULL,
             updated_at = NOW()
         WHERE app_id = $1 AND id = $2
         RETURNING *",
    )
    .bind(app_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

pub async fn decline_price_step_up(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
) -> Result<Subscription, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "UPDATE pay.subscriptions
         SET google_requires_price_step_up_consent = false,
             google_price_step_up_consent_status = 'rejected',
             google_price_step_up_consent_deadline = NULL,
             google_pending_cancellation = true,
             google_pending_cancellation_at = NOW(),
             auto_renewing = false,
             cancellation_initiated_at = NOW(),
             updated_at = NOW()
         WHERE app_id = $1 AND id = $2
         RETURNING *",
    )
    .bind(app_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

pub async fn lookup_user_by_subscription_id(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    subscription_id: &str,
) -> Result<Option<String>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.subscriptions WHERE app_id = $1 AND provider = $2 AND subscription_id = $3 LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(subscription_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn lookup_user_by_purchase_token(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    purchase_token: &str,
) -> Result<Option<String>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.subscriptions WHERE app_id = $1 AND provider = $2 AND purchase_token = $3 LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(purchase_token)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn lookup_subscription_id_by_purchase_token(
    pool: &PgPool,
    app_id: Uuid,
    purchase_token: &str,
) -> Result<Option<String>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT subscription_id FROM pay.subscriptions WHERE app_id = $1 AND purchase_token = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(purchase_token)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

enum SubscriptionFilter<'a> {
    SubscriptionId {
        subscription_id: &'a str,
    },
    SubscriptionIdForProvider {
        provider: &'a str,
        subscription_id: &'a str,
    },
    SubscriptionIdAndUser {
        subscription_id: &'a str,
        external_user_id: &'a str,
    },
    SubscriptionIdAndUserForProvider {
        provider: &'a str,
        subscription_id: &'a str,
        external_user_id: &'a str,
    },
    PurchaseToken {
        purchase_token: &'a str,
    },
    PurchaseTokenForProvider {
        provider: &'a str,
        purchase_token: &'a str,
    },
}

async fn find_subscription(
    pool: &PgPool,
    app_id: Uuid,
    filter: SubscriptionFilter<'_>,
) -> Result<Option<Subscription>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT * FROM pay.subscriptions WHERE app_id = "
    );
    query.push_bind(app_id);

    match filter {
        SubscriptionFilter::SubscriptionId { subscription_id } => {
            query.push(" AND subscription_id = ");
            query.push_bind(subscription_id);
        }
        SubscriptionFilter::SubscriptionIdForProvider {
            provider,
            subscription_id,
        } => {
            query.push(" AND provider = ");
            query.push_bind(provider);
            query.push(" AND subscription_id = ");
            query.push_bind(subscription_id);
        }
        SubscriptionFilter::SubscriptionIdAndUser {
            subscription_id,
            external_user_id,
        } => {
            query.push(" AND subscription_id = ");
            query.push_bind(subscription_id);
            query.push(" AND external_user_id = ");
            query.push_bind(external_user_id);
        }
        SubscriptionFilter::SubscriptionIdAndUserForProvider {
            provider,
            subscription_id,
            external_user_id,
        } => {
            query.push(" AND provider = ");
            query.push_bind(provider);
            query.push(" AND subscription_id = ");
            query.push_bind(subscription_id);
            query.push(" AND external_user_id = ");
            query.push_bind(external_user_id);
        }
        SubscriptionFilter::PurchaseToken { purchase_token } => {
            query.push(" AND purchase_token = ");
            query.push_bind(purchase_token);
        }
        SubscriptionFilter::PurchaseTokenForProvider {
            provider,
            purchase_token,
        } => {
            query.push(" AND provider = ");
            query.push_bind(provider);
            query.push(" AND purchase_token = ");
            query.push_bind(purchase_token);
        }
    }

    let subscription = query
        .build_query_as::<Subscription>()
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(subscription)
}

pub async fn get_subscription_by_sub_id_and_user(
    pool: &PgPool,
    app_id: Uuid,
    subscription_id: &str,
    external_user_id: &str,
) -> Result<Option<Subscription>, BridgeError> {
    find_subscription(
        pool,
        app_id,
        SubscriptionFilter::SubscriptionIdAndUser {
            subscription_id,
            external_user_id,
        },
    )
    .await
}

pub async fn get_subscription_by_sub_id_and_user_for_provider(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    subscription_id: &str,
    external_user_id: &str,
) -> Result<Option<Subscription>, BridgeError> {
    find_subscription(
        pool,
        app_id,
        SubscriptionFilter::SubscriptionIdAndUserForProvider {
            provider,
            subscription_id,
            external_user_id,
        },
    )
    .await
}

pub async fn get_subscription_by_sub_id(
    pool: &PgPool,
    app_id: Uuid,
    subscription_id: &str,
) -> Result<Option<Subscription>, BridgeError> {
    find_subscription(
        pool,
        app_id,
        SubscriptionFilter::SubscriptionId { subscription_id },
    )
    .await
}

pub async fn get_subscription_by_sub_id_for_provider(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    subscription_id: &str,
) -> Result<Option<Subscription>, BridgeError> {
    find_subscription(
        pool,
        app_id,
        SubscriptionFilter::SubscriptionIdForProvider {
            provider,
            subscription_id,
        },
    )
    .await
}

pub async fn get_subscription_by_purchase_token(
    pool: &PgPool,
    app_id: Uuid,
    purchase_token: &str,
) -> Result<Option<Subscription>, BridgeError> {
    find_subscription(
        pool,
        app_id,
        SubscriptionFilter::PurchaseToken { purchase_token },
    )
    .await
}

pub async fn get_subscription_by_purchase_token_for_provider(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    purchase_token: &str,
) -> Result<Option<Subscription>, BridgeError> {
    find_subscription(
        pool,
        app_id,
        SubscriptionFilter::PurchaseTokenForProvider {
            provider,
            purchase_token,
        },
    )
    .await
}

/// Reconciliation write-back keyed on the concrete row id, not subscription_id.
///
/// Google Play subscription_id is a shared product SKU, so two users on the
/// same product have rows that share (app_id, subscription_id). The reconciler
/// already iterates a concrete DB row, so we mutate that exact row by
/// (app_id, id) to avoid clobbering another user's same-SKU row. The
/// high-water stale guard (last_event_time < event_time_ms) is preserved, and
/// duplicate workers skip rows that another worker already corrected.
pub async fn update_reconciled_subscription_status(
    pool: &PgPool,
    app_id: Uuid,
    id: Uuid,
    new_status: &str,
    current_period_end: Option<DateTime<Utc>>,
    event_time_ms: i64,
) -> Result<bool, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let result = sqlx::query(
        "UPDATE pay.subscriptions
         SET status = $1,
             current_period_end = COALESCE($2, current_period_end),
             version = version + 1,
             last_event_time = $3,
             updated_at = NOW()
         WHERE app_id = $4
           AND id = $5
           AND status IS DISTINCT FROM $1
           AND last_event_time < $3"
    )
    .bind(new_status)
    .bind(current_period_end)
    .bind(event_time_ms)
    .bind(app_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() > 0)
}

pub async fn lookup_user_by_google_obfuscated_id(
    pool: &PgPool,
    app_id: Uuid,
    obfuscated_id: &str,
) -> Result<Option<String>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.subscriptions WHERE app_id = $1 AND google_obfuscated_account_id = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(obfuscated_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn link_replacement_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    current_subscription_id: &str,
    last_event_time: i64,
) -> Result<(), BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    link_replacement_subscriptions_tx(
        &mut tx,
        app_id,
        external_user_id,
        current_subscription_id,
        last_event_time,
    )
    .await?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn link_replacement_subscriptions_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    current_subscription_id: &str,
    last_event_time: i64,
) -> Result<(), BridgeError> {
    sqlx::query(
        "UPDATE pay.subscriptions
         SET status = 'replaced',
             last_event_time = $4,
             updated_at = NOW()
         WHERE app_id = $1
           AND external_user_id = $2
           AND subscription_id != $3
           AND status IN ('active', 'trial', 'past_due', 'on_hold')
           AND provider = 'google_play'
           AND last_event_time < $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(current_subscription_id)
    .bind(last_event_time)
    .execute(&mut **tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

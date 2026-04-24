use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubscriptionLookupSnapshot {
    pub id: Uuid,
    pub external_user_id: String,
    pub provider: String,
    pub subscription_id: String,
    pub purchase_token: Option<String>,
    pub status: String,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub auto_renewing: Option<bool>,
    pub revocation_reason: Option<String>,
    pub last_event_time: i64,
}

#[derive(Debug, Clone)]
pub struct UserSubscriptionCancellationSnapshot {
    pub subscription_id: String,
    pub provider: String,
    pub purchase_token: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SubscriptionWebhookTransition {
    Pending,
    GracePeriod {
        grace_period_end: Option<chrono::DateTime<chrono::Utc>>,
    },
    Revoked {
        revocation_reason: Option<String>,
    },
    OnHold,
    Paused,
    Resumed {
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    },
    CancellationScheduled {
        google_cancellation_context: Option<String>,
        google_cancellation_feedback: Option<String>,
    },
    Expired,
    Cancelled {
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
        google_cancellation_context: Option<String>,
        google_cancellation_feedback: Option<String>,
    },
    PaymentFailed,
    PendingPurchaseCancelled,
    PriceStepUp {
        google_new_price_cents: Option<i32>,
        google_price_step_up_consent_deadline: Option<chrono::DateTime<chrono::Utc>>,
    },
    PauseScheduled {
        google_pause_scheduled_at: chrono::DateTime<chrono::Utc>,
    },
    Deferred {
        google_deferred_until: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct WebhookPaymentRecordRequest<'a> {
    pub app_id: Uuid,
    pub external_user_id: &'a str,
    pub provider: &'a str,
    pub provider_transaction_id: &'a str,
    pub subscription_id: Option<&'a str>,
    pub product_id: Option<&'a str>,
    pub amount_cents: i32,
    pub status: &'a str,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionCommitRequest<'a> {
    pub app_id: Uuid,
    pub external_user_id: &'a str,
    pub subscription_id: &'a str,
    pub provider: &'a str,
    pub status: &'a str,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub purchase_token: Option<&'a str>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub provider_customer_id: Option<&'a str>,
    pub event_time_ms: i64,
    pub payment: Option<WebhookPaymentRecordRequest<'a>>,
    pub adopt_stale_payment: bool,
    pub stale_payment_window_secs: i64,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionSnapshot {
    pub purchase_token: Option<String>,
    pub status: String,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub auto_renewing: Option<bool>,
    pub revocation_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebhookProviderSnapshot {
    pub provider: String,
    pub provider_webhook_id: String,
    pub event_type: String,
    pub subscription_id: Option<String>,
    pub purchase_token: Option<String>,
    pub payload: serde_json::Value,
    pub timestamp_epoch_ms: Option<i64>,
    pub suppressed: bool,
    pub suppressed_reason: Option<String>,
}

#[derive(Debug)]
pub enum TransactionOutcome<T> {
    Commit(T),
    Rollback(T),
}

#[derive(Debug, Clone)]
pub struct OwnedWebhookPaymentRecord {
    pub app_id: Uuid,
    pub external_user_id: String,
    pub provider: String,
    pub provider_transaction_id: String,
    pub subscription_id: Option<String>,
    pub product_id: Option<String>,
    pub amount_cents: i32,
    pub status: String,
}

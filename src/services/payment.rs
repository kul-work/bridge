use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Token validation mode for mobile stores
/// TODO: Move to google_play module when it's fully ported
/// Used by Google Play validation module (imported but not directly instantiated).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenValidationMode {
    #[default]
    Strict,  // Full validation
    Relaxed, // Basic validation only
    Off,     // No validation
}

/// Placeholder for Google Play-specific models
/// TODO: Move to google_play module when it's fully ported
pub mod google_play_models {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct OutOfAppPurchaseContext {
        pub expired_subscriptions: Vec<String>,
    }
}

/// Subscription status (normalized across all providers)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscriptionStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "past_due")]
    PastDue,
    #[serde(rename = "unpaid")]
    Unpaid,
    #[serde(rename = "trial")]
    Trial,
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "revoked")]
    Revoked,
    #[serde(rename = "on_hold")]
    OnHold,
    #[serde(rename = "paused")]
    Paused,
}

impl std::fmt::Display for SubscriptionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SubscriptionStatus::Active => "active",
            SubscriptionStatus::Expired => "expired",
            SubscriptionStatus::Cancelled => "cancelled",
            SubscriptionStatus::PastDue => "past_due",
            SubscriptionStatus::Unpaid => "unpaid",
            SubscriptionStatus::Trial => "trial",
            SubscriptionStatus::Pending => "pending",
            SubscriptionStatus::Revoked => "revoked",
            SubscriptionStatus::OnHold => "on_hold",
            SubscriptionStatus::Paused => "paused",
        };
        write!(f, "{}", s)
    }
}

impl From<&str> for SubscriptionStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => SubscriptionStatus::Active,
            "expired" => SubscriptionStatus::Expired,
            "cancelled" => SubscriptionStatus::Cancelled,
            "past_due" => SubscriptionStatus::PastDue,
            "unpaid" => SubscriptionStatus::Unpaid,
            "trial" => SubscriptionStatus::Trial,
            "pending" => SubscriptionStatus::Pending,
            "revoked" => SubscriptionStatus::Revoked,
            "on_hold" => SubscriptionStatus::OnHold,
            "paused" => SubscriptionStatus::Paused,
            _ => SubscriptionStatus::Expired,
        }
    }
}

/// Purchase type for idempotent acknowledgment
/// Used by Google Play lifecycle management (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseType {
    Subscription,
    OneTimeProduct,
}

/// Checkout session returned by payment provider
/// Used by Google Play provider trait methods (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CheckoutSession {
    pub redirect_url: String,
    pub session_id: String,
}

/// PAUSED STATE CONTEXT
/// Tracks metadata about subscription pauses (scheduled or active)
/// Used by Google Play lifecycle management (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct PausedStateContext {
    /// When does pause take effect? (from Type 11 webhook)
    pub pause_scheduled_at: Option<DateTime<Utc>>,

    /// When was subscription actually paused? (from Type 10 webhook)
    pub paused_at: Option<DateTime<Utc>>,

    /// Optional reason user provided for pause (from Type 11)
    pub pause_reason: Option<String>,

    /// Did user manually resume (vs auto-resume)? Set on Type 7 recovery
    pub user_initiated_resume: Option<bool>,
}

/// NORMALIZED SUBSCRIPTION DATA (provider-agnostic, used by all providers)
/// This is the "source of truth" for business logic.
/// All providers map to these fields; business logic never reads provider-specific fields.
/// Used by Google Play subscription lifecycle (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct NormalizedSubscriptionData {
    // Core subscription state
    pub status: SubscriptionStatus,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_renewing: Option<bool>,

    // Cancellation / Revocation (distinct concepts)
    pub cancellation_initiated_at: Option<DateTime<Utc>>,
    pub revocation_reason: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Default for NormalizedSubscriptionData {
    fn default() -> Self {
        Self {
            status: SubscriptionStatus::Pending,
            current_period_end: None,
            auto_renewing: None,
            cancellation_initiated_at: None,
            revocation_reason: None,
            revoked_at: None,
        }
    }
}

/// GOOGLE PLAY RAW DATA (provider-specific, isolated with prefix)
/// Raw data extracted directly from Google Play API v3.
/// All fields are read-only from Google; we store as-is for audit trail.
/// Transformation to normalized data happens in separate function.
/// Used by Google Play provider (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct GooglePlayRawData {
    // Google-specific enums (raw from API, not transformed)
    pub google_subscription_state: i32, // 0-6 enum, raw from API
    pub google_payment_state: Option<i32>, // 0-4 enum, legacy v1
    pub google_cancel_reason: Option<i32>, // 0-1 enum, legacy v1

    // Google-specific identifiers
    pub google_purchase_token: Option<String>,
    pub google_obfuscated_account_id: Option<String>,
    pub google_obfuscated_profile_id: Option<String>,

    // Account linking (resubscribe tracking)
    pub google_linked_purchase_token: Option<String>,
    pub google_previous_subscription_id: Option<String>,

    // Grace period
    pub google_grace_period_start: Option<DateTime<Utc>>,
    pub google_grace_period_end: Option<DateTime<Utc>>,

    // Cancellation / Revocation
    pub google_cancellation_context: Option<String>,
    pub google_cancellation_feedback: Option<String>,

    // Installment subscriptions
    pub google_initial_committed_payments: Option<i32>,
    pub google_remaining_committed_payments: Option<i32>,
    pub google_pending_cancellation: bool,
    pub google_pending_cancellation_at: Option<DateTime<Utc>>,

    // Deferral / Promotional Extension
    pub google_deferred_until: Option<DateTime<Utc>>,

    // Prepaid plans
    pub google_prepaid_ack_deadline: Option<DateTime<Utc>>,
    pub google_prepaid_allow_extend_after: Option<DateTime<Utc>>,
    pub google_prepaid_linked_purchase_token: Option<String>,

    // Price step-up consent (Korea-specific)
    pub google_requires_price_step_up_consent: bool,
    pub google_price_step_up_consent_status: Option<String>,
    pub google_price_step_up_consent_deadline: Option<DateTime<Utc>>,
    pub google_new_price_cents: Option<i32>,
    pub google_is_manual_resume: Option<bool>,
    // Event ordering metadata (from RTDN payload).
    pub google_event_time_millis: Option<i64>,
}

impl Default for GooglePlayRawData {
    fn default() -> Self {
        Self {
            google_subscription_state: 6, // UNKNOWN
            google_payment_state: None,
            google_cancel_reason: None,
            google_purchase_token: None,
            google_obfuscated_account_id: None,
            google_obfuscated_profile_id: None,
            google_linked_purchase_token: None,
            google_previous_subscription_id: None,
            google_grace_period_start: None,
            google_grace_period_end: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_initial_committed_payments: None,
            google_remaining_committed_payments: None,
            google_pending_cancellation: false,
            google_pending_cancellation_at: None,
            google_deferred_until: None,
            google_prepaid_ack_deadline: None,
            google_prepaid_allow_extend_after: None,
            google_prepaid_linked_purchase_token: None,
            google_requires_price_step_up_consent: false,
            google_price_step_up_consent_status: None,
            google_price_step_up_consent_deadline: None,
            google_new_price_cents: None,
            google_is_manual_resume: None,
            google_event_time_millis: None,
        }
    }
}

/// SUBSCRIPTION RECORD (combined for DB storage)
/// Forces developers to populate both normalized AND provider-specific data.
/// Option<GooglePlayRawData> makes it explicit: this is provider-optional data.
/// Used by Google Play lifecycle (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubscriptionRecord {
    // Identifiers
    pub clerk_id: String,
    pub subscription_id: String,
    pub provider: String, // "google_play", "creem", "coinbase"
    pub provider_customer_id: Option<String>,

    // Amount in cents (for payment recording)
    #[allow(dead_code)]
    pub amount_cents: Option<i32>,

    // Normalized (required for all providers)
    pub normalized: NormalizedSubscriptionData,

    // Provider-specific (only populated if applicable)
    pub google_data: Option<GooglePlayRawData>,
    // Unix timestamp millis used for chronological update guards.
    pub last_event_time: i64,
}

/// Webhook event from payment provider
/// Used by Google Play webhook parsing (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct WebhookEvent {
    pub event_id: Option<String>, // Webhook event ID for idempotency (e.g., "evt_xxx")
    pub event_time_millis: Option<i64>, // Provider event timestamp used for ordering
    pub event_type: String,       // e.g. "order.completed", "subscription.updated", etc.
    pub subscription_id: Option<String>,
    pub customer_email: String,
    pub status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub amount_cents: Option<i32>, // Payment amount in cents (e.g., 2999 = $29.99)
    pub provider_customer_id: Option<String>, // Customer ID from payment provider
    pub metadata_user_id: Option<String>, // Internal user ID from provider metadata (e.g., Creem checkout metadata)
    pub purchase_token: Option<String>,
    pub payment_state: Option<i32>,
    pub cancel_reason: Option<i32>,
    pub auto_renewing: Option<bool>,
    // Additional fields
    pub subscription_state: Option<i32>, // Google Play subscription_state (0-6 enum)
    pub grace_period_expiration: Option<DateTime<Utc>>, // Grace period end time
    pub deferred_until: Option<DateTime<Utc>>, // Deferred renewal until this time
    pub obfuscated_account_id: Option<String>, // For account linking
    pub provider_transaction_id: Option<String>, // Provider-specific transaction ID (e.g., Creem last_transaction_id)
}

/// Google Play-specific fields returned by the provider during verification.
/// Isolated here so non-Google providers don't carry irrelevant None fields.
/// Used by Google Play provider (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct GooglePlayProviderData {
    /// For re-subscriptions: the old/expired subscription's token (from Google API)
    pub linked_purchase_token: Option<String>,
    /// Obfuscated account ID from external_account_identifiers
    pub obfuscated_account_id: Option<String>,
    /// Price change details
    pub price_change_new_price_cents: Option<i32>,
    pub price_change_state: Option<String>,
    /// Out-of-app purchase context: expired subscription identifiers for resubscription linking
    pub out_of_app_purchase_context: Option<google_play_models::OutOfAppPurchaseContext>,
    /// SUB-PAUSE: Pause scheduling
    pub pause_scheduled_at: Option<DateTime<Utc>>,
}

/// Provider-specific data attached to a SubscriptionDetails.
/// Used by Google Play provider (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub enum ProviderData {
    GooglePlay(GooglePlayProviderData),
    #[default]
    None,
}

/// Subscription details from provider
/// Used by Google Play provider (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubscriptionDetails {
    pub subscription_id: String,
    pub status: SubscriptionStatus,
    pub customer_email: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub purchase_token: Option<String>,
    pub payment_state: Option<i32>,
    pub cancel_reason: Option<i32>,
    pub auto_renewing: Option<bool>,
    /// Amount in cents (e.g., 2999 = $29.99). Issue #3: Price parsing for mobile/Google Play.
    pub amount_cents: Option<i32>,
    #[allow(dead_code)]
    /// When the purchase was acknowledged with the provider (for 3-day rule compliance)
    /// db update_subscription_acknowledged_at() is doing that
    pub acknowledged_at: Option<DateTime<Utc>>,
    /// Provider-specific data (Google Play fields, etc.)
    pub provider_data: ProviderData,
}

#[allow(dead_code)]
impl SubscriptionDetails {
    /// Access Google Play-specific data if present.
    /// Used by google_play provider integration.
    
    pub fn google_play(&self) -> Option<&GooglePlayProviderData> {
        match &self.provider_data {
            ProviderData::GooglePlay(data) => Some(data),
            ProviderData::None => None,
        }
    }

    // Convenience accessors for frequently used Google Play fields
    /// Used by google_play provider integration.
    pub fn google_obfuscated_account_id(&self) -> Option<&str> {
        self.google_play().and_then(|g| g.obfuscated_account_id.as_deref())
    }
    /// Used by google_play provider integration.
    pub fn google_linked_purchase_token(&self) -> Option<&str> {
        self.google_play().and_then(|g| g.linked_purchase_token.as_deref())
    }
    /// Used by google_play provider integration.
    pub fn google_out_of_app_purchase_context(&self) -> Option<&google_play_models::OutOfAppPurchaseContext> {
        self.google_play().and_then(|g| g.out_of_app_purchase_context.as_ref())
    }
    /// Used by google_play provider integration.
    pub fn google_pause_scheduled_at(&self) -> Option<DateTime<Utc>> {
        self.google_play().and_then(|g| g.pause_scheduled_at)
    }
}

/// Result of a token verification, supporting account linking flows.
/// Used by Google Play provider (imported in type signature).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// The token was successfully verified and linked to the current user.
    Success(Box<SubscriptionDetails>),
    /// The token is valid, but belongs to a different existing user.
    /// The client should prompt the user to log in to the correct account.
    LinkingRequired { obfuscated_account_id: String },
}



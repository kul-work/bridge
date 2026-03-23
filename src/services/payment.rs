use crate::error::AppError;
use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Token validation mode for mobile stores
/// TODO: Move to google_play module when it's fully ported
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
/// Used for future Google Play purchase acknowledgment flow
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseType {
    Subscription,
    OneTimeProduct,
}

/// Checkout session returned by payment provider
/// Used for future provider-specific checkout implementations
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CheckoutSession {
    pub redirect_url: String,
    pub session_id: String,
}

/// PAUSED STATE CONTEXT
/// Tracks metadata about subscription pauses (scheduled or active)
/// Currently used only for documentation; Phase 2 will integrate this fully
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
/// Used for future webhook processing when multiple providers are implemented.
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
/// Used for future webhook processing and Google Play integration.
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
/// Used for future subscription state management and provider integration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubscriptionRecord {
    // Identifiers
    pub clerk_id: String,
    pub subscription_id: String,
    pub provider: String, // "google_play", "stripe", "lemonsqueezy", "creem"
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
/// Used for future webhook ingress and processing across multiple providers.
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
/// Used for future Google Play webhook processing and subscription details.
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
/// Used for future provider-specific subscription detail queries.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ProviderData {
    GooglePlay(GooglePlayProviderData),
    None,
}

impl Default for ProviderData {
    fn default() -> Self { ProviderData::None }
}

/// Subscription details from provider
/// Used for future subscription verification and detail queries across providers.
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

impl SubscriptionDetails {
    /// Access Google Play-specific data if present.
    #[allow(dead_code)]
    pub fn google_play(&self) -> Option<&GooglePlayProviderData> {
        match &self.provider_data {
            ProviderData::GooglePlay(data) => Some(data),
            ProviderData::None => None,
        }
    }

    // Convenience accessors for frequently used Google Play fields
    #[allow(dead_code)]
    pub fn google_obfuscated_account_id(&self) -> Option<&str> {
        self.google_play().and_then(|g| g.obfuscated_account_id.as_deref())
    }
    #[allow(dead_code)]
    pub fn google_linked_purchase_token(&self) -> Option<&str> {
        self.google_play().and_then(|g| g.linked_purchase_token.as_deref())
    }
    #[allow(dead_code)]
    pub fn google_out_of_app_purchase_context(&self) -> Option<&google_play_models::OutOfAppPurchaseContext> {
        self.google_play().and_then(|g| g.out_of_app_purchase_context.as_ref())
    }
    #[allow(dead_code)]
    pub fn google_pause_scheduled_at(&self) -> Option<DateTime<Utc>> {
        self.google_play().and_then(|g| g.pause_scheduled_at)
    }
}

/// Result of a token verification, supporting account linking flows.
/// Used for future account linking and resubscription flows.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// The token was successfully verified and linked to the current user.
    Success(Box<SubscriptionDetails>),
    /// The token is valid, but belongs to a different existing user.
    /// The client should prompt the user to log in to the correct account.
    LinkingRequired { obfuscated_account_id: String },
}

/// Generic payment provider interface
/// Defines the contract for payment provider implementations.
/// Many methods are not currently used but are part of the architectural design.
#[allow(dead_code)]
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Create a checkout session for a user
    async fn create_checkout(
        &self,
        user_id: &str,
        email: &str,
        product_type: Option<&str>,
    ) -> Result<CheckoutSession, AppError>;

    /// Verify webhook authenticity and parse event
    async fn verify_and_parse_webhook(
        &self,
        body: &[u8],
        signature: &str,
        headers: &HeaderMap,
    ) -> Result<WebhookEvent, AppError>;

    /// Get subscription details
    #[allow(dead_code)]
    async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<SubscriptionDetails, AppError>;

    /// Cancel subscription
    async fn cancel_subscription(&self, subscription_id: &str) -> Result<(), AppError>;

    /// Cancel subscription with provider-specific mode (e.g., immediate/scheduled)
    /// Default implementation delegates to cancel_subscription.
    async fn cancel_subscription_with_mode(
        &self,
        subscription_id: &str,
        _mode: Option<&str>,
        _on_execute: Option<&str>,
    ) -> Result<(), AppError> {
        self.cancel_subscription(subscription_id).await
    }

    /// Cancel subscription with purchase token (for Google Play)
    /// Default implementation delegates to cancel_subscription for backward compatibility
    async fn cancel_subscription_with_token(
        &self,
        subscription_id: &str,
        _purchase_token: Option<&str>,
    ) -> Result<(), AppError> {
        self.cancel_subscription(subscription_id).await
    }

    /// Create customer billing portal URL (if supported by provider).
    async fn create_billing_portal(&self, _customer_id: &str) -> Result<String, AppError> {
        Err(AppError::PaymentProviderError(
            "Billing portal not supported by this provider".to_string(),
        ))
    }

    /// Resume a subscription (if supported by provider).
    async fn resume_subscription(&self, _subscription_id: &str) -> Result<(), AppError> {
        Err(AppError::PaymentProviderError(
            "Resume subscription not supported by this provider".to_string(),
        ))
    }

    /// Get provider name
    #[allow(dead_code)]
    fn provider_name(&self) -> &'static str;

    /// Get the header name for webhook signature verification
    fn signature_header_name(&self) -> &'static str;

    /// Get the header name for webhook ID (idempotency)
    fn webhook_id_header_name(&self) -> &'static str;

    /// Verify a purchase token (for mobile stores)
    /// user_id: Authenticated user ID for user binding validation
    /// validation_mode: Token validation mode (Strict, Relaxed, Off)
    async fn verify_token(
        &self,
        _token: &str,
        _subscription_id: &str,
        _product_type: Option<String>,
        _user_id: Option<&str>,
        _validation_mode: crate::services::payment::TokenValidationMode,
    ) -> Result<VerificationResult, AppError> {
        Err(AppError::PaymentProviderError(
            "Token verification not supported by this provider".to_string(),
        ))
    }

    /// Enrich and normalize a webhook event with provider-specific data.
    /// Called during subscription activation to fill in missing fields (e.g., period_end from API)
    /// and normalize status values. Default is a no-op for providers that don't need it.
    async fn enrich_webhook_event(
        &self,
        _clerk_id: &str,
        _subscription_id: &str,
        _event: &mut WebhookEvent,
        _mock_fixture_path: &Option<String>,
    ) {
        // Default: no enrichment needed (Creem, LemonSqueezy)
    }

    /// For downcasting to concrete types (used internally for provider-specific operations)
    #[allow(dead_code)]
    fn as_any(&self) -> &dyn std::any::Any;

    /// Acknowledge a purchase (subscription or OTP) with the provider
    ///
    /// **Important**: This method is NOT idempotent at the provider level. Idempotency must be
    /// enforced by the application layer (handlers/service) by checking `acknowledged_at` in the
    /// database before calling this method.
    ///
    /// **Parameters**:
    /// - `subscription_id`: The subscription/product ID from the provider (e.g., Google Play subscription ID)
    /// - `purchase_token`: The purchase token for this purchase (unique identifier from provider)
    /// - `purchase_type`: Type of purchase (Subscription or OneTimeProduct)
    /// - `user_id`: Optional user ID (not used for provider acknowledgment, kept for signature compatibility)
    ///
    /// **Returns**: `Ok(())` if acknowledgment succeeded.
    ///
    /// **When to use**:
    /// - Only after the application layer has verified idempotency (checked `acknowledged_at`)
    /// - For Google Play compliance: acknowledgment must happen within 3 days, otherwise auto-refund
    ///
    /// **Provider-specific behavior**:
    /// - **GooglePlay**: Calls acknowledge_subscription() or acknowledge() API. Returns error if already acknowledged or token invalid.
    /// - **Creem/LemonSqueezy**: No-op (these providers don't require acknowledgment).
    async fn acknowledge_purchase_idempotent(
        &self,
        subscription_id: &str,
        purchase_token: &str,
        purchase_type: PurchaseType,
        user_id: Option<&str>,
    ) -> Result<(), AppError> {
        // Default implementation: provider does not support acknowledgment
        let _ = (subscription_id, purchase_token, purchase_type, user_id);
        Ok(())
    }
}

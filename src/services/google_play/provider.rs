/// Google Play billing provider implementation
/// Handles payment verification, subscription management, and webhook parsing
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::fs;

use crate::error::AppError;
use crate::services::payment::{
    CheckoutSession, GooglePlayProviderData, PaymentProvider, ProviderData, PurchaseType,
    SubscriptionDetails, SubscriptionStatus, WebhookEvent,
};

use super::client::GooglePlayClient;
use super::models::{DeveloperNotification, PubSubMessage};
use super::validation::{TokenValidationMode, TokenValidator};

pub struct GooglePlayProvider {
    pub client: GooglePlayClient,
    pub package_name: String,
    api_mock: bool,
    verify_webhook_signature: bool,
}

impl GooglePlayProvider {
    fn money_to_cents(money: &super::models::Money) -> Option<i32> {
        let units_raw = match money.units.as_deref() {
            Some(value) => value,
            None => {
                tracing::warn!("Missing Money.units in Google Play price payload");
                return None;
            }
        };

        let units = match units_raw.parse::<i64>() {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!(
                    "Failed to parse Money.units '{}' as i64 in Google Play price payload: {}",
                    units_raw,
                    e
                );
                return None;
            }
        };

        if units < 0 {
            tracing::warn!(
                "Negative Money.units '{}' in Google Play price payload, ignoring",
                units
            );
            return None;
        }

        let nanos_raw = i64::from(money.nanos.unwrap_or(0));
        let nanos = nanos_raw.clamp(0, 999_999_999);
        if nanos != nanos_raw {
            tracing::warn!(
                "Out-of-range Money.nanos '{}' in Google Play price payload, clamped to '{}'",
                nanos_raw,
                nanos
            );
        }

        let cents_from_units = match units.checked_mul(100) {
            Some(value) => value,
            None => {
                tracing::warn!(
                    "Overflow while converting Money.units '{}' to cents in Google Play price payload",
                    units
                );
                return None;
            }
        };

        // Explicitly truncate sub-cent nanos to preserve existing behavior.
        let cents_from_nanos = nanos / 10_000_000;
        let total_cents = match cents_from_units.checked_add(cents_from_nanos) {
            Some(value) => value,
            None => {
                tracing::warn!(
                    "Overflow while adding nanos-derived cents for Money.units='{}', Money.nanos='{}'",
                    units,
                    nanos
                );
                return None;
            }
        };

        if total_cents > i32::MAX as i64 {
            tracing::warn!(
                "Money cents '{}' exceeds i32::MAX in Google Play price payload, ignoring",
                total_cents
            );
            return None;
        }

        Some(total_cents as i32)
    }

    pub fn new(
        package_name: String,
        service_account_path: String,
        verify_audience: bool,
        pub_sub_audience: String,
        skip_rsa_verification: bool,
        api_mock: bool,
        verify_webhook_signature: bool,
    ) -> Result<Self, AppError> {
        let client = GooglePlayClient::with_config(
            &service_account_path,
            verify_audience,
            pub_sub_audience,
            skip_rsa_verification,
        )
        .map_err(|e| {
            AppError::ConfigError(format!("Failed to create Google Play client: {}", e))
        })?;
        Ok(Self {
            client,
            package_name,
            api_mock,
            verify_webhook_signature,
        })
    }

    fn load_fixture_from_path<T: DeserializeOwned>(path: &str) -> Option<T> {
        let content = match fs::read_to_string(path) {
            Ok(val) => val,
            Err(e) => {
                tracing::warn!("Failed to read fixture file {}: {}", path, e);
                return None;
            }
        };

        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(payload) = value.get("payload") {
                if let Ok(parsed) = serde_json::from_value::<T>(payload.clone()) {
                    tracing::info!("Loaded fixture payload from {}", path);
                    return Some(parsed);
                }
            }
        }

        match serde_json::from_str::<T>(&content) {
            Ok(val) => {
                tracing::info!("Loaded fixture from {}", path);
                Some(val)
            }
            Err(e) => {
                tracing::warn!("Failed to parse fixture file {}: {}", path, e);
                None
            }
        }
    }

    fn load_fixture_from_env<T: DeserializeOwned>(env_key: &str) -> Option<T> {
        let path = match std::env::var(env_key) {
            Ok(val) => val,
            Err(_) => return None,
        };

        Self::load_fixture_from_path(&path)
    }

    pub async fn verify_token_with_fixture(
        &self,
        fixture_path: Option<String>,
        token: &str,
        subscription_id: &str,
        product_type: Option<String>,
        user_id: Option<&str>,
        validation_mode: TokenValidationMode,
    ) -> Result<crate::services::payment::VerificationResult, AppError> {
        if fixture_path.is_some() && self.api_mock {
            return self
                .verify_token_with_fixture_path(
                    fixture_path,
                    token,
                    subscription_id,
                    product_type,
                    user_id,
                    validation_mode,
                )
                .await;
        }

        self.verify_token(token, subscription_id, product_type, user_id, validation_mode)
            .await
    }

    async fn verify_token_with_fixture_path(
        &self,
        fixture_path: Option<String>,
        token: &str,
        subscription_id: &str,
        product_type: Option<String>,
        user_id: Option<&str>,
        validation_mode: TokenValidationMode,
    ) -> Result<crate::services::payment::VerificationResult, AppError> {
        // Only apply fixture in mock mode; otherwise delegate
        if !self.api_mock {
            return self
                .verify_token(token, subscription_id, product_type, user_id, validation_mode)
                .await;
        }

        let fixture_path = fixture_path.or_else(|| std::env::var("MOCK_GOOGLE_PURCHASE_RESPONSE").ok());

        if product_type.as_deref() == Some("inapp") {
            TokenValidator::apply_validation(validation_mode, token, subscription_id, self.api_mock)?;

            if let Some(path) = fixture_path.as_deref() {
                if let Some(purchase) =
                    Self::load_fixture_from_path::<super::models::ProductPurchase>(path)
                {
                    let mock_json = serde_json::to_string(&purchase).unwrap_or_default();
                    tracing::info!(target: "BPT-RAW", "GooglePlay Raw Response - get_product: {}", mock_json);

                    let status = match purchase.purchase_state {
                        0 => SubscriptionStatus::Active,
                        1 => SubscriptionStatus::Cancelled,
                        2 => SubscriptionStatus::Pending,
                        _ => SubscriptionStatus::Unknown(format!("purchase_state:{}", purchase.purchase_state)),
                    };

                    // acknowledgement_state: 1 = ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED
                    let acknowledged_at = match purchase.acknowledgement_state {
                        Some(1) => Some(Utc::now()),
                        _ => None,
                    };

                    return Ok(crate::services::payment::VerificationResult::Success(Box::new(
                        SubscriptionDetails {
                            subscription_id: subscription_id.to_string(),
                            status,
                            customer_email: "".to_string(),
                            current_period_end: None,
                            purchase_token: Some(token.to_string()),
                            payment_state: Some(purchase.purchase_state),
                            cancel_reason: None,
                            auto_renewing: None,
                            amount_cents: None,
                            acknowledged_at,
                            provider_data: ProviderData::GooglePlay(GooglePlayProviderData {
                                obfuscated_account_id: purchase.obfuscated_account_id.clone(),
                                ..Default::default()
                            }),
                        },
                    )));
                }
            }

            return self
                .verify_token(token, subscription_id, product_type, user_id, validation_mode)
                .await;
        }

        TokenValidator::apply_validation(validation_mode, token, subscription_id, self.api_mock)?;

        if let Some(path) = fixture_path.as_deref() {
            if let Some(purchase) =
                Self::load_fixture_from_path::<super::models::SubscriptionPurchaseV2>(path)
            {
                let mock_json = serde_json::to_string(&purchase).unwrap_or_default();
                tracing::info!(target: "BPT-RAW", "GooglePlay Raw Response - get_subscription (v2): {}", mock_json);
                return self
                    .verify_subscription_from_purchase(
                        purchase,
                        token,
                        subscription_id,
                        user_id,
                        validation_mode,
                    )
                    .await;
            }
        }

        self.verify_token(token, subscription_id, product_type, user_id, validation_mode)
            .await
    }

    async fn verify_subscription_from_purchase(
        &self,
        purchase: super::models::SubscriptionPurchaseV2,
        token: &str,
        subscription_id: &str,
        user_id: Option<&str>,
        _validation_mode: TokenValidationMode,
    ) -> Result<crate::services::payment::VerificationResult, AppError> {
        // Reuse existing subscription processing flow by emulating the post-fetch logic.
        let google_obfuscated_account_id = purchase
            .external_account_identifiers
            .as_ref()
            .and_then(|ids| ids.obfuscated_account_id.clone());

        if let Some(ref obfuscated_id) = google_obfuscated_account_id {
            tracing::debug!(
                "Current subscription obfuscated_account_id: {}",
                obfuscated_id
            );
        }

        // Check for out-of-app purchase context (resubscription linking)
        if let Some(oap_ctx) = &purchase.out_of_app_purchase_context {
            if let Some(expired_identifiers) = &oap_ctx.expired_external_account_identifiers {
                if let Some(expired_id) = &expired_identifiers.obfuscated_account_id {
                    let should_signal_linking = match user_id {
                        Some(current_user_id) => {
                            let current_hash = Self::compute_obfuscated_id_hash(current_user_id);
                            current_hash != *expired_id
                        }
                        None => true,
                    };

                    if should_signal_linking {
                        return Ok(
                            crate::services::payment::VerificationResult::LinkingRequired {
                                obfuscated_account_id: expired_id.clone(),
                            },
                        );
                    }
                }
            }
        }

        // Check for mismatch between current user and subscription owner (account linking required)
        // This handles the case where a different user tries to claim an existing subscription.
        // The subscription has an obfuscated_account_id (owner's hash), and if the current user's
        // computed hash doesn't match, they need to link their account to the original owner's subscription.
        if let Some(ref owner_hash) = google_obfuscated_account_id {
            if let Some(current_user_id) = user_id {
                let current_user_hash = Self::compute_obfuscated_id_hash(current_user_id);
                if current_user_hash != *owner_hash {
                    // Different user attempting to access this subscription - require linking
                    return Ok(
                        crate::services::payment::VerificationResult::LinkingRequired {
                            obfuscated_account_id: owner_hash.clone(),
                        },
                    );
                }
            }
        }

        let expiry_str = purchase.expiry_time.as_deref()
            .or_else(|| purchase.line_items.first().and_then(|li| li.expiry_time.as_deref()));

        let expires_at = if let Some(expiry_str) = expiry_str {
            chrono::DateTime::parse_from_rfc3339(expiry_str)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    tracing::error!("Invalid ISO expiry timestamp: {}: {}", expiry_str, e);
                    AppError::PaymentProviderError(
                        "Invalid subscription expiry timestamp format".to_string(),
                    )
                })?
        } else {
            Utc::now()
        };

        let status = if let Some(state_str) = &purchase.subscription_state {
            match state_str.as_str() {
                "SUBSCRIPTION_STATE_ACTIVE" => SubscriptionStatus::Active,
                "SUBSCRIPTION_STATE_CANCELED" => SubscriptionStatus::Cancelled,
                "SUBSCRIPTION_STATE_IN_GRACE_PERIOD" => SubscriptionStatus::PastDue,
                "SUBSCRIPTION_STATE_ON_HOLD" => SubscriptionStatus::OnHold,
                "SUBSCRIPTION_STATE_PAUSED" => SubscriptionStatus::Paused,
                "SUBSCRIPTION_STATE_PENDING" => SubscriptionStatus::Pending,
                "SUBSCRIPTION_STATE_EXPIRED" => SubscriptionStatus::Expired,
                _ => SubscriptionStatus::Unknown(state_str.clone()),
            }
        } else {
            SubscriptionStatus::Unknown("no_subscription_state".to_string())
        };

        let auto_renewing = purchase.auto_renewing.or_else(|| {
            purchase.line_items.first()
                .and_then(|li| li.auto_renewing_plan.as_ref())
                .and_then(|plan| plan.auto_renew_enabled)
        });

        let needs_ack = purchase.acknowledgement_state.as_deref()
            == Some("ACKNOWLEDGEMENT_STATE_PENDING")
            || purchase.acknowledgement_state.is_none();

        let acknowledged_at = if needs_ack {
            Some(Utc::now())
        } else {
            None
        };

        let mut price_change_new_price = None;
        let mut price_change_state = None;
        if let Some(summary) = &purchase.price_change_summary {
            if let Some(new_price) = &summary.new_price {
                price_change_new_price = Self::money_to_cents(new_price);
            }
            price_change_state = summary.price_change_state.clone();
        }

        Ok(crate::services::payment::VerificationResult::Success(Box::new(
            SubscriptionDetails {
                subscription_id: subscription_id.to_string(),
                status,
                customer_email: "".to_string(),
                current_period_end: Some(expires_at),
                purchase_token: Some(token.to_string()),
                payment_state: None,
                cancel_reason: None,
                auto_renewing,
                amount_cents: None,
                acknowledged_at,
                provider_data: ProviderData::GooglePlay(GooglePlayProviderData {
                    linked_purchase_token: purchase.linked_purchase_token.clone(),
                    obfuscated_account_id: google_obfuscated_account_id,
                    price_change_new_price_cents: price_change_new_price,
                    price_change_state: price_change_state,
                    out_of_app_purchase_context: purchase.out_of_app_purchase_context.clone(),
                    pause_scheduled_at: None,
                }),
            },
        )))
    }

    /// Get subscription details with optional fixture override
    ///
    /// Used in webhook processing to enrich cancellation data with cancellation feedback
    pub async fn get_subscription_with_fixture(
        &self,
        package_name: &str,
        subscription_id: &str,
        token: &str,
        fixture_path: Option<String>,
    ) -> Result<super::models::SubscriptionPurchaseV2, AppError> {
        if self.api_mock {
            if let Some(path) = fixture_path {
                if let Some(purchase) = Self::load_fixture_from_path::<super::models::SubscriptionPurchaseV2>(&path) {
                    let json = serde_json::to_string_pretty(&purchase).unwrap_or_default();
                    tracing::debug!(target: "BPT-RAW", "GooglePlay Raw Response - get_subscription (v2): {}", json);
                    return Ok(purchase);
                }
                tracing::warn!("Mock mode: fixture path provided but failed to load, returning empty mock");
            } else {
                tracing::info!("MOCK: No fixture path for get_subscription_with_fixture, returning empty mock");
            }
            // Mock mode without valid fixture: return empty SubscriptionPurchaseV2 instead of calling real API
            return Ok(super::models::SubscriptionPurchaseV2::default());
        }

        // Real API call (non-mock mode only)
        self.client.get_subscription(package_name, subscription_id, token).await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to get subscription: {}", e)))
    }

    /// Acknowledge subscription via Google Play API (Resubscribe linking)
    ///
    /// **GOOGLE MOCK CALL**: Simulates acknowledging subscription purchases.
    /// - **Why needed**: HLD requires acknowledging new subscriptions within 3 days (5 min for testers).
    ///   Failure to acknowledge triggers automatic refund by Google.
    /// - **Test coverage**: ACK-01, ACK-02 (initial purchase & resubscription acknowledgement)
    /// - **Mock behavior**: Returns success immediately (no actual Google API call in dev mode)
    pub async fn acknowledge_subscription(
        &self,
        subscription_id: &str,
        purchase_token: &str,
    ) -> Result<(), AppError> {
        if self.api_mock {
            tracing::info!(
                "MOCK: Simulating acknowledge_subscription for subscription_id: {}, token: {}",
                subscription_id,
                purchase_token
            );
            tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - acknowledge_subscription (mock): {{}}");
            return Ok(());
        }

        self.client
            .acknowledge_subscription(&self.package_name, subscription_id, purchase_token)
            .await
            .map_err(|e| {
                AppError::PaymentProviderError(format!("Failed to acknowledge subscription: {}", e))
            })
    }

    /// Acknowledge purchase (subscription or one-time product) via Google Play API
    ///
    /// **GOOGLE MOCK CALL**: Simulates acknowledging purchases.
    /// - **Why needed**: HLD requires acknowledging purchases within 3 days (5 min for testers).
    ///   Failure to acknowledge triggers automatic refund by Google.
    /// - **Test coverage**: OTP-01 (one-time purchase acknowledgement), OTP-RTDN-01 (webhook-based verification)
    /// - **Mock behavior**: Returns success immediately (no actual Google API call in dev mode)
    pub async fn acknowledge(
        &self,
        product_id: &str,
        purchase_token: &str,
    ) -> Result<(), AppError> {
        if self.api_mock {
            tracing::info!(
                "MOCK: Simulating acknowledge for product_id: {}, token: {}",
                product_id,
                purchase_token
            );
            tracing::info!(target: "BPT_RAW", "GooglePlay Raw Response - acknowledge (mock): {{}}");
            return Ok(());
        }

        self.client
            .acknowledge(&self.package_name, product_id, purchase_token)
            .await
            .map_err(|e| {
                AppError::PaymentProviderError(format!("Failed to acknowledge purchase: {}", e))
            })
    }

    /// Compute obfuscated external account ID from user ID (SHA256 hash).
    /// This matches the hash Google Play computes for user binding validation.
    /// Reference: https://developer.android.com/google/play/billing/obfuscated-ids
    /// User binding validation
    fn compute_obfuscated_id_hash(user_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(user_id.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Validate that the obfuscated external account ID from the purchase matches the user's ID.
    /// This ensures the purchase belongs to the authenticated user (security critical).
    /// User binding validation - Now enforced strictly
    fn validate_user_binding(
        user_id: &str,
        obfuscated_id_from_api: Option<&str>,
    ) -> Result<(), AppError> {
        let expected_hash = Self::compute_obfuscated_id_hash(user_id);

        match obfuscated_id_from_api {
            Some(api_hash) => {
                if api_hash != expected_hash {
                    tracing::error!(
                        "User binding validation FAILED - API hash mismatch for user {} (expected: {}, got: {})",
                        user_id,
                        expected_hash,
                        api_hash
                    );
                    Err(AppError::PaymentProviderError(
                        "Purchase does not belong to the authenticated user (binding mismatch)"
                            .to_string(),
                    ))
                } else {
                    tracing::info!("User binding validation PASSED for user {}", user_id);
                    Ok(())
                }
            }
            None => {
                // Strict enforcement - obfuscatedExternalAccountId is REQUIRED in production
                // This prevents unauthorized purchases if user binding is tampered with
                tracing::error!(
                    "User binding validation FAILED - API did not return obfuscatedExternalAccountId for user {}. \
                     Possible causes: (1) Old Google API version, (2) Testing with mock, (3) Account not properly bound in Play Console",
                    user_id
                );
                Err(AppError::PaymentProviderError(
                    "User binding validation failed: obfuscatedExternalAccountId required but not provided by API".to_string()
                ))
            }
        }
    }

    /// Google Play Store: Map subscription notification_type to standard event names
    /// Reference: https://developer.android.com/google/play/billing/rtdn-reference
    /// NOTE: Subscription-only mapping. DO NOT use for OTP notifications (use map_otp_notification_type instead).
    /// Unimplemented: no handlers in webhooks.rs yet
    fn map_subscription_notification_type_to_event(notification_type: i32) -> String {
        match notification_type {
            // Subscription events
            1 => "subscription.recovered".to_string(),                  // SUBSCRIPTION_RECOVERED
            2 => "subscription.paid".to_string(),                       // SUBSCRIPTION_RENEWED
            3 => "subscription.cancelled".to_string(),                  // SUBSCRIPTION_CANCELED
            4 => "subscription.created".to_string(),                    // SUBSCRIPTION_PURCHASED
            5 => "subscription.on_hold".to_string(),                    // SUBSCRIPTION_ON_HOLD
            6 => "subscription.grace_period".to_string(),               // SUBSCRIPTION_IN_GRACE_PERIOD
            7 => "subscription.restarted".to_string(),                  // SUBSCRIPTION_RESTARTED
            8 => "subscription.price_changed".to_string(),              // SUBSCRIPTION_PRICE_CHANGE_CONFIRMED
            9 => "subscription.deferred".to_string(),                   // Unimplemented - SUBSCRIPTION_DEFERRED (dev-initiated extension; call get_subscription() for new expiryTime; no code-cover need it)
            10 => "subscription.paused".to_string(),                    // SUBSCRIPTION_PAUSED
            11 => "subscription.pause_scheduled".to_string(),           // SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED
            12 => "subscription.revoked".to_string(),                   // SUBSCRIPTION_REVOKED
            13 => "subscription.expired".to_string(),                   // SUBSCRIPTION_EXPIRED
            17 => "subscription.items_changed".to_string(),             // Unimplemented - SUBSCRIPTION_ITEMS_CHANGED (bundle)
            18 => "subscription.cancellation_scheduled".to_string(),    // Unimplemented - SUBSCRIPTION_CANCELLATION_SCHEDULED
            19 => "subscription.price_change_updated".to_string(),      // SUBSCRIPTION_PRICE_CHANGE_UPDATED
            20 => "subscription.pending_purchase_canceled".to_string(), // SUBSCRIPTION_PENDING_PURCHASE_CANCELED
            // Price step-up consent (Korea-specific regulatory requirement)
            22 => "subscription.price_step_up_consent_updated".to_string(), // SUBSCRIPTION_PRICE_STEP_UP_CONSENT_UPDATED
            _ => format!(
                "google.subscription.notification_type.{}",
                notification_type
            ),
        }
    }

    fn is_price_change_subscription_notification(notification_type: i32) -> bool {
        matches!(notification_type, 8 | 19 | 22)
    }

    /// Google Play Store: Map one-time product notification_type to (event_type, status) tuple
    /// Context-specific OTP mapping with status derivation.
    /// Returns normalized event name and associated status for database state tracking.
    fn map_otp_notification_type(notification_type: i32) -> (String, &'static str) {
        match notification_type {
            // One-Time Product events
            1 => ("one_time_product.purchased".to_string(), "active"),     // ONE_TIME_PRODUCT_PURCHASED
            2 => ("purchase.voided".to_string(), "refunded"),              // OTP REFUND - no constant event attached
            14 => ("one_time_product.canceled".to_string(), "cancelled"),  // ONE_TIME_PRODUCT_CANCELED
            _ => (
                format!(
                    "google.one_time_product.notification_type.{}",
                    notification_type
                ),
                "unknown",
            ),
        }
    }

    async fn verify_inapp_token(
        &self,
        token: &str,
        subscription_id: &str,
        product_type: Option<String>,
        user_id: Option<&str>,
        validation_mode: TokenValidationMode,
    ) -> Result<crate::services::payment::VerificationResult, AppError> {
            // Mock mode: skip actual Google API call
            if self.api_mock {
                // Apply token validation based on mode
                TokenValidator::apply_validation(validation_mode, token, subscription_id, self.api_mock)?;

                if let Some(purchase) =
                    Self::load_fixture_from_env::<super::models::ProductPurchase>(
                        "MOCK_GOOGLE_PURCHASE_RESPONSE",
                    )
                {
                    let mock_json = serde_json::to_string(&purchase).unwrap_or_default();
                    tracing::debug!(
                        target: "BPT-RAW",
                        "GooglePlay Raw Response - get_product: {}",
                        mock_json
                    );

                    let status = match purchase.purchase_state {
                        0 => SubscriptionStatus::Active,
                        1 => SubscriptionStatus::Cancelled,
                        2 => SubscriptionStatus::Pending,
                        _ => SubscriptionStatus::Unknown(format!("purchase_state:{}", purchase.purchase_state)),
                    };

                    let acknowledged_at = match purchase.acknowledgement_state {
                        Some(1) => Some(Utc::now()),
                        _ => None,
                    };

                    return Ok(crate::services::payment::VerificationResult::Success(Box::new(
                        SubscriptionDetails {
                            subscription_id: subscription_id.to_string(),
                            status,
                            customer_email: "".to_string(),
                            current_period_end: None,
                            purchase_token: Some(token.to_string()),
                            payment_state: Some(purchase.purchase_state),
                            cancel_reason: None,
                            auto_renewing: None,
                            amount_cents: None,
                            acknowledged_at,
                            provider_data: ProviderData::GooglePlay(GooglePlayProviderData {
                                obfuscated_account_id: purchase.obfuscated_account_id.clone(),
                                ..Default::default()
                            }),
                        },
                    )));
                }

                // Support test scenarios: tokens with "declined" or "fail" simulate payment decline
                if token.contains("declined") || token.contains("fail") {
                    tracing::info!("MOCK_EXTERNAL_APIS: Simulating declined INAPP product");
                    tracing::debug!("Token: {}", token);
                    return Err(AppError::PaymentProviderError(
                        "Payment was declined by the payment method".to_string(),
                    ));
                }

                // Support PENDING test: tokens with "slow" or "pending" suffix simulate slow card approval
                if token.contains("slow") || token.contains("pending") {
                    tracing::info!(
                        "MOCK_EXTERNAL_APIS: Simulating PENDING INAPP product for token: {}",
                        token
                    );
                    return Ok(crate::services::payment::VerificationResult::Success(Box::new(
                        SubscriptionDetails {
                            subscription_id: subscription_id.to_string(),
                            status: SubscriptionStatus::Pending,
                            customer_email: "".to_string(),
                            current_period_end: None,
                            purchase_token: Some(token.to_string()),
                            payment_state: Some(2), // Pending
                            cancel_reason: None,
                            auto_renewing: None,
                            amount_cents: None,
                            acknowledged_at: None, // Not acknowledged yet (pending)
                            provider_data: ProviderData::GooglePlay(GooglePlayProviderData::default()),
                        },
                    )));
                }

                tracing::info!(
                    "MOCK_EXTERNAL_APIS: Returning stubbed INAPP product verification for token: {}",
                    token
                );
                tracing::info!("MOCK: Simulating INAPP product acknowledgement");
                tracing::debug!("Token: {}", token);
                return Ok(crate::services::payment::VerificationResult::Success(Box::new(
                    SubscriptionDetails {
                        subscription_id: subscription_id.to_string(),
                        status: SubscriptionStatus::Active,
                        customer_email: "".to_string(),
                        current_period_end: None,
                        purchase_token: Some(token.to_string()),
                        payment_state: Some(0), // Purchased
                        cancel_reason: None,
                        auto_renewing: None,
                        amount_cents: None,                // 0 as mocked
                        acknowledged_at: Some(Utc::now()), // Mock acknowledgement
                        provider_data: ProviderData::GooglePlay(GooglePlayProviderData::default()),
                    },
                )));
            }

            let purchase = self
                .client
                .get_product(&self.package_name, subscription_id, token)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to verify Google INAPP product: {}", e);
                    AppError::PaymentProviderError(format!("Verification failed: {}", e))
                })?;

            // Enforce user binding validation via obfuscatedExternalAccountId (subscriptions only)
            // This ensures the purchase belongs to the authenticated user (security critical)
            // NOTE: Google Play only returns obfuscatedExternalAccountId for subscriptions (product_type: "sub")
            // One-time products (product_type: "inapp") don't have this field, so we skip validation for them
            if let Some(authenticated_user_id) = user_id {
                // Only enforce strict binding validation for subscriptions
                if let Some("sub") = product_type.as_deref() {
                    Self::validate_user_binding(
                        authenticated_user_id,
                        purchase.obfuscated_account_id.as_deref(),
                    )?;
                } else {
                    // For inapp products, binding validation is optional since API doesn't return the field
                    tracing::debug!("Skipping user binding validation: inapp product (obfuscatedAccountId only for subscriptions)");
                }
            } else {
                tracing::warn!(
                    "User binding validation skipped: user_id not provided (optional in trait)"
                );
            }

            // Map purchase state to status
            // purchaseState: 0 (Purchased), 1 (Canceled), 2 (Pending)
            // Handle PENDING state - don't grant access yet
            let status = match purchase.purchase_state {
                0 => SubscriptionStatus::Active,
                1 => SubscriptionStatus::Cancelled,
                2 => SubscriptionStatus::Pending, // Slow card/pre-order - await webhook confirmation
                _ => SubscriptionStatus::Unknown(format!("purchase_state:{}", purchase.purchase_state)),
            };

            // One-time products don't expire (lifetime access), so set expiration to None.
            // purchase_state 0 (Purchased) = permanent access until explicitly refunded/canceled.
            tracing::info!(
                "INAPP product verified: product_id: {}, purchase_state: {}, status: {:?}",
                subscription_id,
                purchase.purchase_state,
                status
            );

            // NOTE: Acknowledgement is handled by the application layer (handlers.rs::verify_purchase)
            // This method is pure verification only - no side effects.

            Ok(crate::services::payment::VerificationResult::Success(Box::new(
                SubscriptionDetails {
                    subscription_id: subscription_id.to_string(),
                    status,
                    customer_email: "".to_string(),
                    current_period_end: None, // Lifetime (no expiration)
                    purchase_token: Some(token.to_string()),
                    payment_state: Some(purchase.purchase_state), // Mapping specific to product (0=Purchased, 1=Canceled, 2=Pending)
                    cancel_reason: None,
                    auto_renewing: None,   // One-time purchases don't auto-renew
                    amount_cents: None, // INAPP pricing not tracked in this response; stored elsewhere in Play Console
                    acknowledged_at: None, // Acknowledgement handled by application layer, not here
                    provider_data: ProviderData::GooglePlay(GooglePlayProviderData::default()),
                },
            )))
    }

    async fn verify_subscription_token(
        &self,
        token: &str,
        subscription_id: &str,
        user_id: Option<&str>,
        validation_mode: TokenValidationMode,
    ) -> Result<crate::services::payment::VerificationResult, AppError> {
        // fallback to Subscription logic (default)
        // For re-subscriptions: extract old token from the new token format.
        // Test format: "test-sub06-new-token-1234" → extract "test-sub06-old-token-1234"
        let linked_token = if token.contains("-new-token-") {
            token.replace("-new-token-", "-old-token-").into()
        } else {
            None
        };

        if linked_token.is_some() {
            tracing::debug!(
                "MOCK/REAL: Extracted linked token from resubscription: {:?}",
                linked_token
            );
        }

        let purchase = if self.api_mock {
            // Apply token validation based on mode
            TokenValidator::apply_validation(validation_mode, token, subscription_id, self.api_mock)?;

            tracing::info!("MOCK_EXTERNAL_APIS: Simulating subscription verification");
            tracing::debug!("Token: {}", token);

            // **GOOGLE MOCK CALL**: Different user attempting to verify another user's purchase token.
            // - **Why needed**: Security test ensuring LinkingRequired prevents unauthorized access via hash mismatch.
            // - **Test coverage**: SUB-19B (LinkingRequired Response - Different Account Verification)
            // - **Mock behavior**: Returns ACTIVE subscription with User1's hash in external_account_identifiers.
            //   When User2 verifies, hash(User2) != User1's hash → LinkingRequired triggered.
            // - **Security**: Prevents unauthorized access to another user's subscription; returns only opaque hash.
            if token == "resubscribe-linking-required" {
                tracing::info!("MOCK_EXTERNAL_APIS: Simulating LinkingRequired scenario (SUB-19B) - Different user verification attempt.");
                let mock_expiry = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
                super::models::SubscriptionPurchaseV2 {
                    kind: Some("androidpublisher#subscriptionPurchaseV2".to_string()),
                    start_time: Some(Utc::now().to_rfc3339()),
                    expiry_time: Some(mock_expiry.clone()),
                    auto_renewing: Some(true),
                    subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
                    latest_order_id: Some("GPA.1234-5678-9012-34567".to_string()),
                    linked_purchase_token: None,
                    acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
                    line_items: vec![super::models::SubscriptionLineItem {
                        product_id: subscription_id.to_string(),
                        expiry_time: Some(mock_expiry),
                        auto_renewing_plan: None,
                        offer_details: None,
                    }],
                    external_account_identifiers: Some(super::models::ExternalAccountIdentifiers {
                        obfuscated_account_id: Some("sub-19b-owner-hash".to_string()),
                        obfuscated_profile_id: None,
                    }),
                    canceled_state_context: None,
                    test_purchase: None,
                    price_change_summary: None,
                    out_of_app_purchase_context: None,
                }
            } else {
                // **GOOGLE MOCK CALL**: Simulates subscription verification (get_subscription API v3).
                // Default happy path mock for a standard active subscription
                // - **Why needed**: Tests subscription lifecycle without calling real Google Play API.
                // - **Test coverage**: SUB-01..SUB-09 (core lifecycle), SUB-14..SUB-21 (trials, price changes, restore)
                // - **Mock behavior**: Smart logic based on token content:
                //   * Parses token keywords (pending, on_hold, paused, cancelled, resubscribe)
                //   * Sets subscription_state accordingly
                //   * For "resubscribe" tokens: Adds external_account_identifiers for linking old purchases
                //   * Returns acknowledgement_state=PENDING to trigger ACK flow (ACK-01, ACK-02, ACK-03)
                //   * Returns user's computed hash as obfuscated_account_id to match subscription ownership
                // - **Default state**: ACTIVE with 30-day expiry, auto_renewing=true
                
                // Compute obfuscated_account_id: use user's hash if authenticated, otherwise token-based
                let obfuscated_account_id = if let Some(current_user_id) = user_id {
                    // User is authenticated: return their hash to indicate they own the subscription
                    Self::compute_obfuscated_id_hash(current_user_id)
                } else {
                    // No authenticated user: use token-based hash for unauthenticated access
                    format!("mock-obfuscated-{}", token.replace("-", "_"))
                };
                
                let mock_expiry = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
                let mock_purchase = super::models::SubscriptionPurchaseV2 {
                    kind: Some("androidpublisher#subscriptionPurchaseV2".to_string()),
                    start_time: Some(Utc::now().to_rfc3339()),
                    expiry_time: Some(mock_expiry.clone()),
                    auto_renewing: Some(
                        !token.contains("cancelled") && !token.contains("canceled"),
                    ),
                    subscription_state: Some(
                        match () {
                            _ if token.contains("pending") => "SUBSCRIPTION_STATE_PENDING",
                            _ if token.contains("on_hold") => "SUBSCRIPTION_STATE_ON_HOLD",
                            _ if token.contains("paused") => "SUBSCRIPTION_STATE_PAUSED",
                            _ if token.contains("cancelled") || token.contains("canceled") => {
                                "SUBSCRIPTION_STATE_CANCELED"
                            }
                            _ => "SUBSCRIPTION_STATE_ACTIVE",
                        }
                        .to_string(),
                    ),
                    latest_order_id: Some("GPA.1234-5678-9012-34567".to_string()),
                    linked_purchase_token: linked_token.clone(),
                    acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()), // Simulate unacknowledged (triggers ack)
                    line_items: vec![super::models::SubscriptionLineItem {
                        product_id: subscription_id.to_string(),
                        expiry_time: Some(mock_expiry),
                        auto_renewing_plan: None,
                        offer_details: None,
                    }],
                    external_account_identifiers: Some(super::models::ExternalAccountIdentifiers {
                        obfuscated_account_id: Some(obfuscated_account_id),
                        obfuscated_profile_id: None,
                    }),
                    canceled_state_context: None,
                    test_purchase: None,
                    price_change_summary: if token.contains("price_change") {
                        Some(super::models::PriceChangeSummary {
                            new_price: Some(super::models::Money {
                                currency_code: Some("USD".to_string()),
                                units: Some("5".to_string()),
                                nanos: Some(0),
                            }),
                            price_change_state: Some("PRICE_CHANGE_STATE_CONFIRMED".to_string()),
                        })
                    } else {
                        None
                    },
                    out_of_app_purchase_context: None,
                };

                // Log mock response in same format as client.rs
                let mock_json = serde_json::to_string(&mock_purchase).unwrap_or_default();
                tracing::debug!(target: "BPT-RAW", "GooglePlay Raw Response - get_subscription (v2): {}", mock_json);

                mock_purchase
            }
        } else {
            TokenValidator::apply_validation(validation_mode, token, subscription_id, self.api_mock)?;
            
            self.client
                .get_subscription(&self.package_name, subscription_id, token) // Updated client uses V2 internal URL but interface keeps sub_id
                .await
                .map_err(|e| {
                    tracing::error!("Failed to verify Google subscription: {}", e);
                    AppError::PaymentProviderError(format!("Verification failed: {}", e))
                })?
        };

        // Extract obfuscated account ID from current subscription
        let google_obfuscated_account_id = purchase
            .external_account_identifiers
            .as_ref()
            .and_then(|ids| ids.obfuscated_account_id.clone());

        if let Some(ref obfuscated_id) = google_obfuscated_account_id {
            tracing::debug!(
                "Current subscription obfuscated_account_id: {}",
                obfuscated_id
            );
        }

        // Handle resubscribe linking BEFORE any other processing.
        // V2: out_of_app_purchase_context contains expired identifiers for resubscription linking
        if let Some(oap_ctx) = &purchase.out_of_app_purchase_context {
            if let Some(expired_identifiers) = &oap_ctx.expired_external_account_identifiers {
                if let Some(expired_id) = &expired_identifiers.obfuscated_account_id {
                    tracing::info!(
                        "Out-of-app purchase context detected. Expired identifier: {}",
                        expired_id
                    );

                    // If the current user (if any) doesn't match the expired ID,
                    // or if there's no user authenticated, signal to the client.
                    let should_signal_linking = match user_id {
                        Some(current_user_id) => {
                            let current_hash = Self::compute_obfuscated_id_hash(current_user_id);
                            current_hash != *expired_id
                        }
                        None => true, // No user logged in, so definitely need to link.
                    };

                    if should_signal_linking {
                        tracing::warn!(
                            "Purchase conflict or unauthenticated resubscribe. Signaling client to initiate account linking for identifier: {}",
                            expired_id
                        );
                        return Ok(
                            crate::services::payment::VerificationResult::LinkingRequired {
                                obfuscated_account_id: expired_id.clone(),
                            },
                        );
                    }

                    // If we are here, it means the currently logged-in user IS the owner. Proceed normally.
                    tracing::info!(
                        "Expired identifier matches current user. Proceeding with verification."
                    );
                }
            }
        }

        // Check for mismatch between current user and subscription owner (account linking required)
        // This handles the case where a different user tries to claim an existing subscription.
        // The subscription has an obfuscated_account_id (owner's hash), and if the current user's
        // computed hash doesn't match, they need to link their account to the original owner's subscription.
        if let Some(ref owner_hash) = google_obfuscated_account_id {
            if let Some(current_user_id) = user_id {
                let current_user_hash = Self::compute_obfuscated_id_hash(current_user_id);
                if current_user_hash != *owner_hash {
                    // Different user attempting to access this subscription - require linking
                    tracing::warn!(
                        "Subscription owner mismatch. Current user hash doesn't match owner hash. Signaling client to initiate account linking for identifier: {}",
                        owner_hash
                    );
                    return Ok(
                        crate::services::payment::VerificationResult::LinkingRequired {
                            obfuscated_account_id: owner_hash.clone(),
                        },
                    );
                }
            }
        }

        // V2 Expiry Parsing (ISO 8601)
        // NOTE: expiryTime is in lineItems[0], not at top level
        let expires_at = if let Some(line_item) = purchase.line_items.first() {
            if let Some(expiry_str) = &line_item.expiry_time {
                chrono::DateTime::parse_from_rfc3339(expiry_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| {
                        tracing::error!("Invalid ISO expiry timestamp from lineItems: {}: {}", expiry_str, e);
                        AppError::PaymentProviderError(
                            "Invalid subscription expiry timestamp format".to_string(),
                        )
                    })?
            } else {
                // Fallback if lineItems[0] has no expiry
                tracing::warn!("Missing expiry in lineItems[0] - subscription will appear expired");
                Utc::now()
            }
        } else {
            // No line items - fallback to now
            tracing::warn!("No lineItems in purchase - subscription will appear expired");
            Utc::now()
        };

        // V2 Status Determination directly from String Enum
        let status = if let Some(state_str) = &purchase.subscription_state {
            match state_str.as_str() {
                "SUBSCRIPTION_STATE_ACTIVE" => SubscriptionStatus::Active,
                "SUBSCRIPTION_STATE_CANCELED" => SubscriptionStatus::Cancelled,
                "SUBSCRIPTION_STATE_IN_GRACE_PERIOD" => SubscriptionStatus::PastDue,
                "SUBSCRIPTION_STATE_ON_HOLD" => SubscriptionStatus::OnHold,
                "SUBSCRIPTION_STATE_PAUSED" => SubscriptionStatus::Paused,
                "SUBSCRIPTION_STATE_PENDING" => SubscriptionStatus::Pending,
                "SUBSCRIPTION_STATE_EXPIRED" => SubscriptionStatus::Expired,
                _ => SubscriptionStatus::Unknown(state_str.clone()),
            }
        } else {
            SubscriptionStatus::Unknown("no_subscription_state".to_string())
        };

        let auto_renewing = purchase.auto_renewing.or_else(|| {
            purchase.line_items.first()
                .and_then(|li| li.auto_renewing_plan.as_ref())
                .and_then(|plan| plan.auto_renew_enabled)
        });

        // Acknowledge the subscription (Lifecycle Requirement)
        // V2 uses string enum: ACKNOWLEDGEMENT_STATE_PENDING or ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED
        let needs_ack = purchase.acknowledgement_state.as_deref()
            == Some("ACKNOWLEDGEMENT_STATE_PENDING")
            || purchase.acknowledgement_state.is_none(); // Assume pending if missing/unknown

        // Track when acknowledgement happened (for 3-day rule audit)
        let acknowledged_at = if needs_ack {
            if self.api_mock {
                // **GOOGLE MOCK CALL**: Simulates ACK inline within verify_payment flow.
                // - **Why needed**: When ACKNOWLEDGEMENT_STATE_PENDING detected, ACK must be called immediately.
                //   This mock prevents Google API call during verification in test mode.
                // - **Test coverage**: ACK-01, ACK-02, ACK-03 (verification-time acknowledgement)
                // - **Mock behavior**: Returns current timestamp immediately WITHOUT calling `self.client.acknowledge_subscription()`.
                //   In production (else branch), the real Google API call is made to acknowledge the subscription.
                // - **Key difference**: Mock = direct return of Utc::now() | Real = async call to Google API then return Utc::now()
                tracing::info!("MOCK: Simulating subscription acknowledgement");
                tracing::debug!("Token: {}", token);
                Some(Utc::now())
            } else {
                if let Err(e) = self
                    .client
                    .acknowledge_subscription(&self.package_name, subscription_id, token)
                    .await
                {
                    tracing::error!("Failed to acknowledge subscription: {}", e);
                    return Err(AppError::PaymentProviderError(format!(
                        "Acknowledgement failed: {}",
                        e
                    )));
                }
                Some(Utc::now())
            }
        } else {
            // Already acknowledged - we don't know when, leave as None
            None
        };

        // V2 Price extraction is complex (in line_items), for now verify logic doesn't strictly require it
        // unless we want to record amount. V2 separates price into base plans/offers.
        // We can skip amount_cents for now or extract from line_items -> offer_details if needed.
        let amount_cents = None;

        // Extract price change summary if available
        let mut price_change_new_price = None;
        let mut price_change_state = None;
        if let Some(summary) = &purchase.price_change_summary {
            if let Some(new_price) = &summary.new_price {
                price_change_new_price = Self::money_to_cents(new_price);
            }
            price_change_state = summary.price_change_state.clone();
        }

        Ok(crate::services::payment::VerificationResult::Success(Box::new(
            SubscriptionDetails {
                subscription_id: subscription_id.to_string(),
                status,
                customer_email: "".to_string(), // Unknown from Google
                current_period_end: Some(expires_at),
                purchase_token: Some(token.to_string()),
                payment_state: None, // V2 doesn't have simple int state
                cancel_reason: None, // V2 uses canceled_state_context
                auto_renewing,
                amount_cents,
                acknowledged_at,
                provider_data: ProviderData::GooglePlay(GooglePlayProviderData {
                    linked_purchase_token: purchase.linked_purchase_token.clone(),
                    obfuscated_account_id: google_obfuscated_account_id,
                    price_change_new_price_cents: price_change_new_price,
                    price_change_state: price_change_state,
                    out_of_app_purchase_context: purchase.out_of_app_purchase_context.clone(),
                    pause_scheduled_at: None,
                }),
            },
        )))
    }
}

#[async_trait]
impl PaymentProvider for GooglePlayProvider {
    async fn create_checkout(
        &self,
        _user_id: &str,
        _email: &str,
        _product_type: Option<&str>,
    ) -> Result<CheckoutSession, AppError> {
        // Google Play billing is initiated on the client device (mobile).
        // This endpoint shouldn't really be used for web checkout in the same way.
        // We return a dummy session or error.
        Err(AppError::PaymentProviderError(
            "Google Play checkout must be initiated on Android device".to_string(),
        ))
    }

    async fn verify_and_parse_webhook(
        &self,
        body: &[u8],
        signature: &str,
        headers: &HeaderMap,
    ) -> Result<WebhookEvent, AppError> {
        // Only allow header overrides in test/mock mode (MOCK_EXTERNAL_APIS=true)
        let verify_signature = if self.api_mock {
            // Priority 1: Request header override (X-Webhook-Verification-Mode: strict/off) - test mode only
            headers
                .get("X-Webhook-Verification-Mode")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_lowercase())
                .map(|mode| match mode.as_str() {
                    "strict" => true,
                    "off" => false,
                    _ => self.verify_webhook_signature,
                })
                // Priority 2: Environment variable
                .unwrap_or(self.verify_webhook_signature)
        } else {
            // Production: always use env config, ignore headers
            self.verify_webhook_signature
        };

        // Only allow header overrides in test/mock mode (MOCK_EXTERNAL_APIS=true)
        let verify_audience = if self.api_mock {
            // Priority 1: Request header override for audience (X-Webhook-Audience-Mode: strict/off) - test mode only
            headers
                .get("X-Webhook-Audience-Mode")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_lowercase())
                .map(|mode| match mode.as_str() {
                    "strict" => true,
                    "off" => false,
                    _ => self.client.verify_aud,
                })
        } else {
            // Production: ignore headers, use env config
            None
        };

        // Use potentially overridden client if audience mode is specified
        let client = if let Some(aud_override) = verify_audience {
            self.client.with_audience_override(aud_override)
        } else {
            self.client.clone()
        };
        let effective_verify_audience = verify_audience.unwrap_or(self.client.verify_aud);
        let strong_webhook_verification = verify_signature && effective_verify_audience;

        // Issue #6: Webhook Signature Verification
        // Verify Google Pub/Sub authentication header if enabled (production: true, dev: can disable via GOOGLE_VERIFY_WEBHOOK_SIGNATURE)
        if verify_signature {
            match client.verify_pubsub_signature(signature).await {
                Ok(true) => {
                    tracing::debug!("Pub/Sub signature verification passed");
                }
                Ok(false) => {
                    tracing::warn!("Pub/Sub signature verification returned false");
                    // Empty signatures now properly rejected
                    return Err(AppError::WebhookVerificationFailed);
                }
                Err(e) => {
                    tracing::error!("Pub/Sub signature verification error: {}", e);
                    return Err(AppError::WebhookVerificationFailed);
                }
            }
        } else {
            tracing::warn!("Pub/Sub signature verification DISABLED (GOOGLE_VERIFY_WEBHOOK_SIGNATURE=false or X-Webhook-Verification-Mode=off)");
        }

        // Try to parse as Pub/Sub message first
        let (dev_notification, pubsub_message_id): (DeveloperNotification, Option<String>) =
            match serde_json::from_slice::<PubSubMessage>(body) {
                Ok(pubsub_msg) => {
                    // Decode base64 data from Pub/Sub wrapper
                    let data_bytes = general_purpose::STANDARD
                        .decode(&pubsub_msg.message.data)
                        .map_err(|e| {
                            tracing::error!("Failed to decode base64 message data: {}", e);
                            AppError::WebhookParseError
                        })?;

                    let notification = serde_json::from_slice(&data_bytes).map_err(|e| {
                        tracing::error!(
                            "Failed to parse DeveloperNotification from Pub/Sub data: {}",
                            e
                        );
                        AppError::WebhookParseError
                    })?;

                    (notification, Some(pubsub_msg.message.message_id.clone()))
                }
                Err(e) => {
                    // Fallback: try parsing as direct DeveloperNotification (for testing or alternative webhook formats)
                    tracing::debug!(
                        parse_error = %e,
                        "Payload did not match Pub/Sub envelope; falling back to direct DeveloperNotification parse"
                    );
                    let notification = serde_json::from_slice(body).map_err(|e2| {
                        tracing::error!("Failed to parse as DeveloperNotification: {}", e2);
                        AppError::WebhookParseError
                    })?;
                    (notification, None)
                }
            };

        // Log package_name from webhook for audit trail
        tracing::debug!(
            "Webhook received - package_name: {}, event_time: {}",
            dev_notification.package_name,
            dev_notification.event_time_millis
        );
        let parsed_event_time_millis = dev_notification.event_time_millis.parse::<i64>().ok();
        // In mock/replay mode fixtures may contain static old timestamps that break
        // deterministic test ordering. Use ingest time for mock mode only.
        let event_time_millis = if self.api_mock {
            Some(Utc::now().timestamp_millis())
        } else {
            parsed_event_time_millis
        };

        // Map Google-specific notification to normalized WebhookEvent
        if let Some(sub_notif) = &dev_notification.subscription_notification {
            if Self::is_price_change_subscription_notification(sub_notif.notification_type)
                && !self.api_mock
                && !strong_webhook_verification
            {
                tracing::warn!(
                    notification_type = sub_notif.notification_type,
                    verify_signature,
                    verify_audience = effective_verify_audience,
                    "Rejecting Google Play price-change webhook due to weak verification settings"
                );
                return Err(AppError::WebhookVerificationFailed);
            }

            let event_type =
                Self::map_subscription_notification_type_to_event(sub_notif.notification_type);
            return Ok(WebhookEvent {
                event_id: pubsub_message_id,
                event_time_millis,
                event_type,
                subscription_id: Some(sub_notif.subscription_id.clone()),
                purchase_token: Some(sub_notif.purchase_token.clone()),
                customer_email: String::new(),
                amount_cents: None,
                current_period_end: None,
                grace_period_expiration: None,
                deferred_until: None,
                payment_state: None,
                cancel_reason: None,
                subscription_state: None,
                auto_renewing: None,
                obfuscated_account_id: None,
                provider_transaction_id: None,
                provider_customer_id: None,
                metadata_user_id: None,
                status: String::new(),
            });
        }

        if let Some(otp_notif) = &dev_notification.one_time_product_notification {
            // OTP refunds come as notificationType: 2 in oneTimeProductNotification
            // (not in voidedPurchaseNotification in all cases)
            // Handle both 2 (refund) and 14 (cancellation) as refund events
            let (event_type, status) = Self::map_otp_notification_type(otp_notif.notification_type);

            return Ok(WebhookEvent {
                event_id: pubsub_message_id,
                event_time_millis,
                event_type,
                subscription_id: Some(otp_notif.product_id.clone()),
                purchase_token: Some(otp_notif.purchase_token.clone()),
                customer_email: String::new(),
                amount_cents: None,
                current_period_end: None,
                grace_period_expiration: None,
                deferred_until: None,
                payment_state: None,
                cancel_reason: None,
                subscription_state: None,
                auto_renewing: None,
                obfuscated_account_id: None,
                provider_transaction_id: None,
                provider_customer_id: None,
                metadata_user_id: None,
                status: status.to_string(),
            });
        }

        // Handle voided purchase notifications (refunds/cancellations)
        if let Some(voided_notif) = &dev_notification.voided_purchase_notification {
            // productType: 0 = OTP, 1 = subscription
            // Use "purchase.voided" for all voided purchases so lookup happens by purchase_token
            // (works for both OTP and subscriptions since both store purchase_token in DB)
            return Ok(WebhookEvent {
                event_id: pubsub_message_id,
                event_time_millis,
                event_type: "purchase.voided".to_string(),
                subscription_id: Some(voided_notif.order_id.clone()), // Kept for logging
                purchase_token: Some(voided_notif.purchase_token.clone()),
                customer_email: String::new(),
                amount_cents: None,
                current_period_end: None,
                grace_period_expiration: None,
                deferred_until: None,
                payment_state: None,
                cancel_reason: Some(voided_notif.refund_type), // 0 = full refund, 1 = partial refund
                subscription_state: None,
                auto_renewing: None,
                obfuscated_account_id: None,
                provider_transaction_id: None,
                provider_customer_id: None,
                metadata_user_id: None,
                status: String::new(),
            });
        }

        Err(AppError::WebhookParseError)
    }

    async fn cancel_subscription(&self, _subscription_id: &str) -> Result<(), AppError> {
        // Google Play doesn't support cancellation via subscription ID alone; need purchase_token
        Err(AppError::PaymentProviderError(
            "Use cancel_subscription_with_token for Google Play".to_string(),
        ))
    }

    async fn cancel_subscription_with_token(
        &self,
        subscription_id: &str,
        purchase_token: Option<&str>,
    ) -> Result<(), AppError> {
        let token = purchase_token.ok_or_else(|| {
            AppError::PaymentProviderError(
                "Google Play requires purchase_token to cancel subscription".to_string(),
            )
        })?;

        self.client
            .cancel_subscription(&self.package_name, subscription_id, token)
            .await
            .map_err(|e| {
                tracing::error!("Failed to cancel Google Play subscription: {}", e);
                AppError::PaymentProviderError(format!("Cancellation failed: {}", e))
            })
    }

    async fn get_subscription(
        &self,
        _subscription_id: &str,
    ) -> Result<crate::services::payment::SubscriptionDetails, AppError> {
        Err(AppError::PaymentProviderError(
            "get_subscription requires purchase_token for Google Play".to_string(),
        ))
    }

    fn provider_name(&self) -> &'static str {
        "google_play"
    }

    fn signature_header_name(&self) -> &'static str {
        "authorization" // Not strictly used for signature but required by trait
    }

    // Source: https://docs.cloud.google.com/pubsub/docs/payload-unwrapping
    fn webhook_id_header_name(&self) -> &'static str {
        "x-goog-pubsub-message-id" // Google Pub/Sub message ID header
    }

    async fn verify_token(
        &self,
        token: &str,
        subscription_id: &str,
        product_type: Option<String>,
        user_id: Option<&str>,
        validation_mode: TokenValidationMode,
    ) -> Result<crate::services::payment::VerificationResult, AppError> {
        if product_type.as_deref() == Some("inapp") {
            self.verify_inapp_token(token, subscription_id, product_type, user_id, validation_mode).await
        } else {
            self.verify_subscription_token(token, subscription_id, user_id, validation_mode).await
        }
    }

    /// Enrich renewal webhooks with full subscription data from Google Play API,
    /// then normalize Google Play subscription_state to a status string.
    async fn enrich_webhook_event(
        &self,
        clerk_id: &str,
        subscription_id: &str,
        event: &mut WebhookEvent,
        mock_fixture_path: &Option<String>,
    ) {
        // --- Enrichment: fill missing period_end from Google Play API ---
        if event.current_period_end.is_none() {
            if let Some(token) = event.purchase_token.clone() {
                tracing::debug!(
                    "Enriching Google Play renewal webhook with subscription data from API for token: {}",
                    token
                );

                let enrich_result = if let Some(ref fixture) = mock_fixture_path {
                    self.verify_token_with_fixture(
                        Some(fixture.clone()),
                        &token,
                        subscription_id,
                        Some("subs".to_string()),
                        Some(clerk_id),
                        TokenValidationMode::Off,
                    ).await
                } else {
                    self.verify_token(
                        &token,
                        subscription_id,
                        Some("subs".to_string()),
                        Some(clerk_id),
                        TokenValidationMode::Off,
                    ).await
                };

                match enrich_result {
                    Ok(crate::services::payment::VerificationResult::Success(details)) => {
                        tracing::debug!(
                            "Successfully enriched renewal webhook - current_period_end: {:?}, status: {:?}",
                            details.current_period_end,
                            details.status
                        );
                        event.current_period_end = details.current_period_end;
                        event.payment_state = details.payment_state;
                        event.auto_renewing = details.auto_renewing;
                        event.amount_cents = details.amount_cents;

                        event.subscription_state = Some(match details.status {
                            SubscriptionStatus::Active => 0,
                            SubscriptionStatus::Cancelled => 1,
                            SubscriptionStatus::PastDue => 2,
                            SubscriptionStatus::Unpaid => 2,
                            SubscriptionStatus::OnHold => 3,
                            SubscriptionStatus::Paused => 4,
                            SubscriptionStatus::Pending => 5,
                            SubscriptionStatus::Trial => 0,
                            SubscriptionStatus::Revoked => 1,
                            SubscriptionStatus::Expired => 6,
                            SubscriptionStatus::Unknown(_) => 6, // treat as expired for Google Play state mapping
                        });
                    }
                    Ok(_) => {
                        tracing::warn!("Webhook enrichment returned non-Success result");
                        tracing::debug!("Token: {}", token);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to enrich renewal webhook from Google Play API: {}. \
                             Proceeding with webhook data (may lack period_end)",
                            e
                        );
                    }
                }
            }
        }

        // --- Normalization: map subscription_state to status string ---
        let google_state = event.subscription_state.unwrap_or(0);
        let grace_period_end = event.grace_period_expiration;
        let deferred_until = event.deferred_until;

        let google_state_str = match google_state {
            0 => "SUBSCRIPTION_STATE_ACTIVE",
            1 => "SUBSCRIPTION_STATE_CANCELED",
            2 => "SUBSCRIPTION_STATE_IN_GRACE_PERIOD",
            3 => "SUBSCRIPTION_STATE_ON_HOLD",
            4 => "SUBSCRIPTION_STATE_PAUSED",
            5 => "SUBSCRIPTION_STATE_PENDING",
            _ => "SUBSCRIPTION_STATE_EXPIRED",
        };

        let normalized = super::subscription_lifecycle::map_google_subscription_state_to_normalized(
            Some(google_state_str),
            event.payment_state,
            event.cancel_reason,
            event.current_period_end,
            grace_period_end,
            deferred_until,
        );

        tracing::debug!(
            "Mapped Google subscription_state {} ({}) to normalized status: {:?}",
            google_state,
            google_state_str,
            normalized.status
        );

        event.status = normalized.status.to_string();
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn acknowledge_purchase_idempotent(
        &self,
        subscription_id: &str,
        purchase_token: &str,
        purchase_type: PurchaseType,
        _user_id: Option<&str>,
    ) -> Result<(), AppError> {
        // Call the appropriate provider API based on purchase type
        // Idempotency is handled at the application/handler layer
        match purchase_type {
            PurchaseType::Subscription => {
                tracing::info!(
                    "Acknowledging subscription: subscription_id={}",
                    subscription_id
                );
                self.acknowledge_subscription(subscription_id, purchase_token)
                    .await?;
            }
            PurchaseType::OneTimeProduct => {
                tracing::info!(
                    "Acknowledging OTP: product_id={}",
                    subscription_id
                );
                self.acknowledge(subscription_id, purchase_token).await?;
            }
        }

        Ok(())
    }
}

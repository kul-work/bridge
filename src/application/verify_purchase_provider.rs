use chrono::{Duration, Utc};
use serde::de::DeserializeOwned;
use std::fs;
use uuid::Uuid;

use crate::application::verify_purchase_types::{
    compute_obfuscated_id_hash, PaymentAcknowledgement, ProductType, VerificationOutcome,
    VerifyPurchaseCallback, VerifiedPurchase,
};
use crate::error::BridgeError;
use crate::ports::{
    AppLookupRepository, WebhookForwardRepository, WebhookWriteRepository,
};
use crate::services::google_play::{
    client::{GoogleOrderPaymentDetails, GooglePlayClient},
    models::{Money, ProductPurchase, SubscriptionPurchaseV2},
};
use crate::webhooks::processor::CanonicalWebhookPayload;

const MOCK_SUBSCRIPTION_TOKEN_PREFIX: &str = "mock-google-play-subscription:";

fn verify_purchase_event_id(
    provider: &str,
    product_type: ProductType,
    subscription_id: &str,
    purchase_token: &str,
) -> String {
    if provider == "google_play" && matches!(product_type, ProductType::OneTimeProduct) {
        let identity = format!("{}:{}", subscription_id, purchase_token);
        return format!("verify-purchase-google-play-otp-{}", crate::utils::diagnostic_hash(&identity));
    }

    format!("verify-purchase-{}", Uuid::new_v4())
}

pub(crate) async fn verify_purchase_with_provider(
    provider: &str,
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
    provider_config: &serde_json::Value,
) -> Result<VerificationOutcome, BridgeError> {
    match provider {
        "google_play" => verify_google_play(
            subscription_id,
            purchase_token,
            product_type,
            external_user_id,
            provider_config,
        )
        .await,
        "apple" => Err(BridgeError::ValidationError(
            "Apple verify-purchase not yet implemented".to_string(),
        )),
        _ => Err(BridgeError::ValidationError(format!(
            "verify-purchase not supported for provider: {} (mobile stores only)",
            provider
        ))),
    }
}

async fn verify_google_play(
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
    config: &serde_json::Value,
) -> Result<VerificationOutcome, BridgeError> {
    if crate::config::mock_external_apis_enabled() {
        return mock_verify_google_play(subscription_id, purchase_token, product_type, external_user_id);
    }

    let client = build_google_play_client(config).await?;

    match product_type {
        ProductType::Subscription => {
            let purchase = client
                .get_subscription(google_package_name(config)?, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play verify failed: {}", e)))?;

            map_google_subscription_verification(purchase, subscription_id, external_user_id)
        }
        ProductType::OneTimeProduct => {
            let purchase = client
                .get_product(google_package_name(config)?, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play verify failed: {}", e)))?;

            let order_payment = match purchase.order_id.as_deref() {
                Some(order_id) => match client
                    .get_order_payment_details(google_package_name(config)?, order_id)
                    .await
                {
                    Ok(order_payment) => order_payment,
                    Err(err) => {
                        tracing::warn!(
                            "Google Play order payment details lookup failed for order {}: {}",
                            order_id,
                            err
                        );
                        GoogleOrderPaymentDetails::default()
                    }
                },
                None => GoogleOrderPaymentDetails::default(),
            };

            Ok(VerificationOutcome::Verified(map_google_product_verification_with_order_payment(
                purchase,
                order_payment,
            )))
        }
    }
}



pub(crate) async fn forward_verify_purchase_callback<
    R: AppLookupRepository + WebhookForwardRepository + WebhookWriteRepository + ?Sized,
>(
    repo: &R,
    app_id: uuid::Uuid,
    app_slug: &str,
    callback: VerifyPurchaseCallback<'_>,
) -> Result<(), BridgeError> {
    let now = chrono::Utc::now();
    let event_id = verify_purchase_event_id(
        &callback.request.provider,
        callback.product_type,
        &callback.request.subscription_id,
        &callback.request.purchase_token,
    );

    let provider_payload = serde_json::json!({
        "source": "verify_purchase",
        "external_user_id": callback.resolved_external_user_id,
        "subscription_id": callback.request.subscription_id,
        "provider": callback.request.provider,
        "status": callback.status,
    });

    let callback_payload = CanonicalWebhookPayload {
        event_id: event_id.clone(),
        event_type: callback.product_type.callback_event_type().to_string(),
        timestamp: now.to_rfc3339(),
        timestamp_epoch_ms: now.timestamp_millis(),
        app_slug: app_slug.to_string(),
        product_id: Some(callback.request.subscription_id.clone()),
        subscription_id: callback.product_type
            .is_subscription()
            .then(|| callback.request.subscription_id.clone()),
        external_user_id: Some(callback.resolved_external_user_id.to_string()),
        amount_cents: callback.amount_cents,
        new_price_cents: None,
        auto_renewing: callback.auto_renewing,
        purchase_token: Some(callback.request.purchase_token.clone()),
        current_period_end: callback.current_period_end.map(|s| s.to_string()),
        status: Some(callback.status.to_string()),
        provider: callback.request.provider.clone(),
        provider_event_id: event_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: None,
        cancellation_mode: None,
        google_price_step_up_consent_deadline: None,
        google_pause_scheduled_at: None,
        google_deferred_until: None,
        google_pending_price_change_new_price_cents: None,
        google_pending_price_change_currency: None,
        google_pending_price_change_mode: None,
        google_pending_price_change_state: None,
        google_pending_price_change_expected_at: None,
    };

    crate::webhooks::forwarding::create_and_forward_webhook(
        repo,
        app_id,
        &callback.request.provider,
        &event_id,
        "verify_purchase.succeeded",
        Some(callback.request.subscription_id.clone()),
        Some(callback.request.purchase_token.clone()),
        provider_payload,
        Some(now.timestamp_millis()),
        callback_payload,
    )
    .await
}

pub(crate) async fn acknowledge_google_play(
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    config: &serde_json::Value,
) -> Result<(), BridgeError> {
    if crate::config::mock_external_apis_enabled() {
        return Ok(());
    }

    let client = build_google_play_client(config).await?;

    match product_type {
        ProductType::Subscription => client
            .acknowledge_subscription(google_package_name(config)?, subscription_id, purchase_token)
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Google Play acknowledgement failed: {}", e))),
        ProductType::OneTimeProduct => client
            .acknowledge(google_package_name(config)?, subscription_id, purchase_token)
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Google Play acknowledgement failed: {}", e))),
    }
}

fn google_package_name(config: &serde_json::Value) -> Result<&str, BridgeError> {
    config
        .get("package_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Google Play package_name".to_string()))
}

async fn build_google_play_client(config: &serde_json::Value) -> Result<GooglePlayClient, BridgeError> {
    let sa_path = config
        .get("service_account_json")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            BridgeError::ConfigError("Missing Google Play service_account_json path".to_string())
        })?;

    let sa_path_owned = sa_path.to_string();
    tokio::task::spawn_blocking(move || GooglePlayClient::new(&sa_path_owned))
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to spawn blocking task: {}", e)))?
        .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))
}



fn google_money_to_cents(money: &Money) -> Option<i32> {
    let units = money.units.as_deref()?.parse::<i64>().ok()?;
    if units < 0 {
        return None;
    }

    let nanos = i64::from(money.nanos.unwrap_or(0)).clamp(0, 999_999_999);
    let total_cents = units.checked_mul(100)?.checked_add(nanos / 10_000_000)?;
    i32::try_from(total_cents).ok()
}

fn map_google_subscription_verification(
    purchase: SubscriptionPurchaseV2,
    subscription_id: &str,
    external_user_id: &str,
) -> Result<VerificationOutcome, BridgeError> {
    let obfuscated_account_id = purchase
        .external_account_identifiers
        .as_ref()
        .and_then(|ids| ids.obfuscated_account_id.clone());

    let resubscribe_obfuscated_account_id = purchase
        .out_of_app_purchase_context
        .as_ref()
        .and_then(|ctx| ctx.expired_external_account_identifiers.as_ref())
        .and_then(|ids| ids.obfuscated_account_id.clone());

    if resubscribe_obfuscated_account_id.is_none() {
        if let Some(owner_hash) = obfuscated_account_id.clone() {
            if compute_obfuscated_id_hash(external_user_id) != owner_hash {
                return Ok(VerificationOutcome::LinkingRequired {
                    obfuscated_account_id: owner_hash,
                });
            }
        }
    }

    if purchase.line_items.is_empty()
        || !purchase
            .line_items
            .iter()
            .any(|line_item| line_item.product_id == subscription_id)
    {
        return Err(BridgeError::ValidationError(
            "Purchase token does not match the requested subscription ID".to_string(),
        ));
    }

    let status = match purchase.subscription_state.as_deref() {
        Some("SUBSCRIPTION_STATE_ACTIVE") | Some("SUBSCRIPTION_STATE_IN_GRACE_PERIOD") => "active",
        Some("SUBSCRIPTION_STATE_CANCELED") => "cancelled",
        Some("SUBSCRIPTION_STATE_ON_HOLD") => "on_hold",
        Some("SUBSCRIPTION_STATE_PAUSED") => "paused",
        Some("SUBSCRIPTION_STATE_EXPIRED") => "expired",
        Some("SUBSCRIPTION_STATE_PENDING") => "pending",
        _ => "expired",
    }
    .to_string();

    let current_period_end = purchase
        .line_items
        .first()
        .and_then(|line_item| line_item.expiry_time.as_deref())
        .or(purchase.expiry_time.as_deref())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let auto_renewing = purchase.auto_renewing.or_else(|| {
        purchase
            .line_items
            .first()
            .and_then(|line_item| line_item.auto_renewing_plan.as_ref())
            .and_then(|plan| plan.auto_renew_enabled)
    });

    let amount_cents = google_subscription_current_amount_cents(&purchase);
    let currency = purchase
        .line_items
        .iter()
        .find_map(|line_item| line_item.auto_renewing_plan.as_ref())
        .and_then(|plan| plan.recurring_price.as_ref())
        .and_then(|money| money.currency_code.clone());

    let acknowledgement = match purchase.acknowledgement_state.as_deref() {
        Some("ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED") => PaymentAcknowledgement::AlreadyAcknowledged,
        _ => PaymentAcknowledgement::Pending,
    };
    let provider_transaction_id = purchase
        .line_items
        .first()
        .and_then(|line_item| line_item.latest_successful_order_id.clone())
        .or_else(|| purchase.latest_order_id.clone());

    Ok(VerificationOutcome::Verified(VerifiedPurchase {
        status,
        provider_transaction_id,
        current_period_end,
        auto_renewing,
        amount_cents,
        currency,
        payment_state: None,
        acknowledgement,
        obfuscated_account_id,
        resubscribe_obfuscated_account_id,
        linked_purchase_token: purchase.linked_purchase_token.clone(),
    }))
}

fn google_subscription_current_amount_cents(purchase: &SubscriptionPurchaseV2) -> Option<i32> {
    let line_item = purchase.line_items.first()?;

    if line_item
        .offer_phase
        .as_ref()
        .and_then(|phase| phase.free_trial.as_ref())
        .is_some()
    {
        return Some(0);
    }

    line_item
        .auto_renewing_plan
        .as_ref()
        .and_then(|plan| plan.recurring_price.as_ref())
        .and_then(google_money_to_cents)
}

fn map_google_product_verification(
    purchase: ProductPurchase,
    amount_cents: Option<i32>,
) -> VerifiedPurchase {
    let status = match purchase.purchase_state {
        0 => "active",
        1 => "cancelled",
        2 => "pending",
        _ => "expired",
    }
    .to_string();

    let acknowledgement = match purchase.acknowledgement_state {
        Some(1) => PaymentAcknowledgement::AlreadyAcknowledged,
        _ => PaymentAcknowledgement::Pending,
    };

    VerifiedPurchase {
        status,
        provider_transaction_id: purchase.order_id,
        current_period_end: None,
        auto_renewing: None,
        amount_cents,
        currency: None,
        payment_state: Some(purchase.purchase_state),
        acknowledgement,
        obfuscated_account_id: purchase.obfuscated_account_id,
        resubscribe_obfuscated_account_id: None,
        linked_purchase_token: None,
    }
}

fn map_google_product_verification_with_order_payment(
    purchase: ProductPurchase,
    order_payment: GoogleOrderPaymentDetails,
) -> VerifiedPurchase {
    let mut verified = map_google_product_verification(purchase, order_payment.amount_cents);
    verified.currency = order_payment.currency;
    verified
}

fn mock_verify_google_play(
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
) -> Result<VerificationOutcome, BridgeError> {
    match product_type {
        ProductType::Subscription => {
            reject_mock_google_subscription_token(subscription_id, purchase_token)?;

            if let Some(purchase) = load_mock_fixture::<SubscriptionPurchaseV2>(
                "MOCK_GOOGLE_PURCHASE_RESPONSE",
            ) {
                return map_google_subscription_verification(purchase, subscription_id, external_user_id);
            }

            // Token designed to test linking_required: returns Verified so the first
            // user becomes the owner; a second user with the same token will hit
            // "token already bound to different user" → linking_required.
            if purchase_token.starts_with("resubscribe-linking-required") {
                let obfuscated_account_id = compute_obfuscated_id_hash(external_user_id);
                return Ok(VerificationOutcome::Verified(VerifiedPurchase {
                    status: "active".to_string(),
                    provider_transaction_id: Some(format!("mock-google-play-order:{}", purchase_token)),
                    current_period_end: Some(Utc::now() + Duration::days(30)),
                    auto_renewing: Some(true),
                    amount_cents: None,
                    currency: None,
                    payment_state: None,
                    acknowledgement: PaymentAcknowledgement::Pending,
                    obfuscated_account_id: Some(obfuscated_account_id),
                    resubscribe_obfuscated_account_id: None,
                    linked_purchase_token: None,
                }));
            }

            if purchase_token.contains("oap") || purchase_token.contains("resubscribe") {
                let obfuscated_account_id = compute_obfuscated_id_hash(external_user_id);
                // Re-subscription: derive linked token from -new-token- naming convention
                let linked_token = if purchase_token.contains("-new-token-") {
                    purchase_token.replace("-new-token-", "-old-token-").into()
                } else {
                    None
                };

                return Ok(VerificationOutcome::Verified(VerifiedPurchase {
                    status: "active".to_string(),
                    provider_transaction_id: Some(format!("mock-google-play-order:{}", purchase_token)),
                    current_period_end: Some(Utc::now() + Duration::days(30)),
                    auto_renewing: Some(true),
                    amount_cents: None,
                    currency: None,
                    payment_state: None,
                    acknowledgement: PaymentAcknowledgement::Pending,
                    obfuscated_account_id: Some(obfuscated_account_id.clone()),
                    resubscribe_obfuscated_account_id: Some(obfuscated_account_id),
                    linked_purchase_token: linked_token,
                }));
            }

            let status = if purchase_token.contains("pending") {
                "pending"
            } else if purchase_token.contains("on_hold") {
                "on_hold"
            } else if purchase_token.contains("paused") {
                "paused"
            } else if purchase_token.contains("cancelled") || purchase_token.contains("canceled") {
                "cancelled"
            } else {
                "active"
            }
            .to_string();

            // Re-subscription: derive linked token from -new-token- naming convention
            let linked_token = if purchase_token.contains("-new-token-") {
                purchase_token.replace("-new-token-", "-old-token-").into()
            } else {
                None
            };

            Ok(VerificationOutcome::Verified(VerifiedPurchase {
                status,
                provider_transaction_id: Some(format!("mock-google-play-order:{}", purchase_token)),
                current_period_end: Some(Utc::now() + Duration::days(30)),
                auto_renewing: Some(
                    !purchase_token.contains("cancelled") && !purchase_token.contains("canceled"),
                ),
                amount_cents: None,
                currency: None,
                payment_state: None,
                acknowledgement: PaymentAcknowledgement::Pending,
                obfuscated_account_id: Some(compute_obfuscated_id_hash(external_user_id)),
                resubscribe_obfuscated_account_id: None,
                linked_purchase_token: linked_token,
            }))
        }
        ProductType::OneTimeProduct => {
            if let Some(purchase) = load_mock_fixture::<ProductPurchase>("MOCK_GOOGLE_PURCHASE_RESPONSE")
            {
                return Ok(VerificationOutcome::Verified(map_google_product_verification(
                    purchase,
                    None,
                )));
            }

            if purchase_token.contains("declined") || purchase_token.contains("fail") {
                return Err(BridgeError::ProviderError(
                    "Payment was declined by the payment method".to_string(),
                ));
            }

            let status = if purchase_token.contains("slow") || purchase_token.contains("pending") {
                "pending"
            } else {
                "active"
            }
            .to_string();

            Ok(VerificationOutcome::Verified(VerifiedPurchase {
                status,
                provider_transaction_id: Some(format!("mock-google-play-order:{}", purchase_token)),
                current_period_end: None,
                auto_renewing: None,
                amount_cents: None,
                currency: None,
                payment_state: Some(if purchase_token.contains("slow") || purchase_token.contains("pending") {
                    2
                } else {
                    0
                }),
                acknowledgement: PaymentAcknowledgement::Pending,
                obfuscated_account_id: None,
                resubscribe_obfuscated_account_id: None,
                linked_purchase_token: None,
            }))
        }
    }
}

fn reject_mock_google_subscription_token(
    subscription_id: &str,
    purchase_token: &str,
) -> Result<(), BridgeError> {
    let token = purchase_token.trim();

    if token.is_empty() {
        return Err(BridgeError::ValidationError(
            "Purchase token cannot be empty".to_string(),
        ));
    }

    if token.len() < 10 {
        return Err(BridgeError::ValidationError(
            "Purchase token is too short (minimum 10 characters)".to_string(),
        ));
    }

    if token.len() > 1000 {
        return Err(BridgeError::ValidationError(
            "Purchase token is too long (maximum 1000 characters)".to_string(),
        ));
    }

    if token.contains("invalid")
        || token.contains("not-a-valid")
        || (token.contains("abc") && token.contains("!@#$"))
        || token.chars().all(|c| c.is_numeric())
    {
        return Err(BridgeError::ValidationError(
            "Invalid or revoked purchase token".to_string(),
        ));
    }

    if token.contains("expired") {
        return Err(BridgeError::ProviderError(
            "Purchase token has expired (>60 days past subscription expiry)".to_string(),
        ));
    }

    if token.contains("api-error") || token.contains("google-api-error") {
        return Err(BridgeError::ProviderError(
            "Google Play API temporarily unavailable (5xx error)".to_string(),
        ));
    }

    if token.contains("mismatch") {
        return Err(BridgeError::ValidationError(
            "Purchase token does not match the requested subscription ID".to_string(),
        ));
    }

    if let Some(token_subscription_id) = mock_subscription_id_from_token(token) {
        if token_subscription_id != subscription_id {
            return Err(BridgeError::ValidationError(
                "Purchase token does not match the requested subscription ID".to_string(),
            ));
        }
    }

    Ok(())
}

fn mock_subscription_id_from_token(token: &str) -> Option<&str> {
    token
        .strip_prefix(MOCK_SUBSCRIPTION_TOKEN_PREFIX)
        .and_then(|rest| rest.split(':').next())
        .filter(|subscription_id| !subscription_id.is_empty())
}

fn load_mock_fixture<T: DeserializeOwned>(env_key: &str) -> Option<T> {
    let path = std::env::var(env_key).ok()?;
    let content = fs::read_to_string(path).ok()?;

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(payload) = value.get("payload") {
            if let Ok(parsed) = serde_json::from_value::<T>(payload.clone()) {
                return Some(parsed);
            }
        }
    }

    serde_json::from_str::<T>(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::google_play::models::{
        AutoRenewingPlan, ExternalAccountIdentifiers, OfferPhase, OutOfAppPurchaseContext, SubscriptionLineItem,
    };

    #[test]
    fn google_subscription_verification_returns_recurring_price_cents() {
        let purchase = SubscriptionPurchaseV2 {
            auto_renewing: Some(true),
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
            line_items: vec![SubscriptionLineItem {
                product_id: "premium_monthly".to_string(),
                expiry_time: Some("2026-04-30T00:00:00Z".to_string()),
                latest_successful_order_id: None,
                auto_renewing_plan: Some(AutoRenewingPlan {
                    auto_renew_enabled: Some(true),
                    recurring_price: Some(Money {
                        currency_code: Some("USD".to_string()),
                        units: Some("12".to_string()),
                        nanos: Some(990_000_000),
                    }),
                    price_change_details: None,
                }),
                offer_details: None,
                offer_phase: None,
            }],
            ..Default::default()
        };

        let verification = map_google_subscription_verification(purchase, "premium_monthly", "user-123")
            .expect("google subscription verification should succeed");

        match verification {
            VerificationOutcome::Verified(verified) => {
                assert_eq!(verified.amount_cents, Some(1299));
                assert_eq!(verified.currency, Some("USD".to_string()));
                assert_eq!(verified.status, "active");
            }
            VerificationOutcome::LinkingRequired { .. } => {
                panic!("expected a verified purchase response")
            }
        }
    }

    #[test]
    fn google_subscription_verification_uses_zero_amount_for_free_trial_phase() {
        let purchase = SubscriptionPurchaseV2 {
            auto_renewing: Some(true),
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
            line_items: vec![SubscriptionLineItem {
                product_id: "premium_monthly".to_string(),
                expiry_time: Some("2026-04-30T00:00:00Z".to_string()),
                latest_successful_order_id: Some("GPA.1234-5678-9012-34567".to_string()),
                auto_renewing_plan: Some(AutoRenewingPlan {
                    auto_renew_enabled: Some(true),
                    recurring_price: Some(Money {
                        currency_code: Some("RON".to_string()),
                        units: Some("5".to_string()),
                        nanos: Some(490_000_000),
                    }),
                    price_change_details: None,
                }),
                offer_details: None,
                offer_phase: Some(OfferPhase {
                    free_trial: Some(serde_json::json!({})),
                    base_price: None,
                }),
            }],
            ..Default::default()
        };

        let verification = map_google_subscription_verification(purchase, "premium_monthly", "user-123")
            .expect("google subscription verification should succeed");

        match verification {
            VerificationOutcome::Verified(verified) => {
                assert_eq!(verified.amount_cents, Some(0));
                assert_eq!(verified.currency, Some("RON".to_string()));
                assert_eq!(verified.status, "active");
            }
            VerificationOutcome::LinkingRequired { .. } => {
                panic!("expected a verified purchase response")
            }
        }
    }

    #[test]
    fn google_subscription_verification_rejects_mismatched_product_id() {
        let purchase = SubscriptionPurchaseV2 {
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
            line_items: vec![SubscriptionLineItem {
                product_id: "premium_monthly".to_string(),
                expiry_time: Some("2026-04-30T00:00:00Z".to_string()),
                latest_successful_order_id: None,
                auto_renewing_plan: None,
                offer_details: None,
                offer_phase: None,
            }],
            ..Default::default()
        };

        let err = match map_google_subscription_verification(
            purchase,
            "wrong_subscription_id",
            "user-123",
        ) {
            Ok(_) => panic!("mismatched subscription id should be rejected"),
            Err(err) => err,
        };

        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("does not match")));
    }

    #[test]
    fn mock_google_subscription_rejects_err_token_patterns() {
        let cases = [
            "invalid-token-xyz",
            "1234567890",
            "not-a-valid-purchase-token",
            "abc!@#$%^&*()",
            "expired-token-err-03",
            "google-api-error-500",
            "mock-google-play-subscription:premium_monthly:token-123",
        ];

        for token in cases {
            assert!(
                reject_mock_google_subscription_token("wrong_subscription_id", token).is_err(),
                "expected token {token} to be rejected"
            );
        }
    }

    #[test]
    fn google_subscription_verification_prefers_expired_obfuscated_account_id_for_resubscribe() {
        let purchase = SubscriptionPurchaseV2 {
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
            line_items: vec![SubscriptionLineItem {
                product_id: "premium_monthly".to_string(),
                expiry_time: None,
                latest_successful_order_id: None,
                auto_renewing_plan: None,
                offer_details: None,
                offer_phase: None,
            }],
            external_account_identifiers: Some(ExternalAccountIdentifiers {
                obfuscated_account_id: Some("current-owner-hash".to_string()),
                obfuscated_profile_id: None,
            }),
            out_of_app_purchase_context: Some(OutOfAppPurchaseContext {
                expired_external_account_identifiers: Some(ExternalAccountIdentifiers {
                    obfuscated_account_id: Some("expired-owner-hash".to_string()),
                    obfuscated_profile_id: None,
                }),
                expired_purchase_token: Some("old-token".to_string()),
            }),
            ..Default::default()
        };

        let verification = map_google_subscription_verification(purchase, "premium_monthly", "user-123")
            .expect("google subscription verification should succeed");

        match verification {
            VerificationOutcome::Verified(verified) => {
                assert_eq!(
                    verified.resubscribe_obfuscated_account_id,
                    Some("expired-owner-hash".to_string())
                );
                assert_eq!(
                    verified.obfuscated_account_id,
                    Some("current-owner-hash".to_string())
                );
            }
            VerificationOutcome::LinkingRequired { .. } => {
                panic!("expected resubscribe verification to continue")
            }
        }
    }

    #[test]
    fn google_subscription_verification_requires_linking_when_owner_hash_differs() {
        let purchase = SubscriptionPurchaseV2 {
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
            external_account_identifiers: Some(ExternalAccountIdentifiers {
                obfuscated_account_id: Some("owner-hash".to_string()),
                obfuscated_profile_id: None,
            }),
            ..Default::default()
        };

        let verification = map_google_subscription_verification(purchase, "premium_monthly", "user-123")
            .expect("google subscription verification should succeed");

        match verification {
            VerificationOutcome::LinkingRequired { obfuscated_account_id } => {
                assert_eq!(obfuscated_account_id, "owner-hash");
            }
            VerificationOutcome::Verified(_) => {
                panic!("expected linking_required for mismatched owner hash")
            }
        }
    }

    #[test]
    fn google_product_verification_uses_enriched_order_amount() {
        let purchase = ProductPurchase {
            kind: "androidpublisher".to_string(),
            purchase_state: 0,
            purchase_time_millis: "1712448000000".to_string(),
            order_id: Some("GPA.1234-5678-9012-34567".to_string()),
            obfuscated_account_id: None,
            acknowledgement_state: Some(1),
        };

        let verified = map_google_product_verification_with_order_payment(
            purchase,
            GoogleOrderPaymentDetails {
                amount_cents: Some(499),
                currency: Some("RON".to_string()),
            },
        );

        assert_eq!(verified.amount_cents, Some(499));
        assert_eq!(verified.currency, Some("RON".to_string()));
        assert_eq!(verified.status, "active");
        assert_eq!(verified.payment_state, Some(0));
    }

    #[test]
    fn google_one_time_verify_event_id_is_stable_per_product_and_token() {
        let first = verify_purchase_event_id(
            "google_play",
            ProductType::OneTimeProduct,
            "hiha_one_time",
            "purchase-token-123",
        );
        let second = verify_purchase_event_id(
            "google_play",
            ProductType::OneTimeProduct,
            "hiha_one_time",
            "purchase-token-123",
        );
        let other_product = verify_purchase_event_id(
            "google_play",
            ProductType::OneTimeProduct,
            "other_one_time",
            "purchase-token-123",
        );

        assert_eq!(first, second);
        assert_ne!(first, other_product);
        assert!(first.starts_with("verify-purchase-google-play-otp-"));
    }

    #[test]
    fn subscription_verify_event_id_remains_per_request() {
        let first = verify_purchase_event_id(
            "google_play",
            ProductType::Subscription,
            "premium_monthly",
            "purchase-token-123",
        );
        let second = verify_purchase_event_id(
            "google_play",
            ProductType::Subscription,
            "premium_monthly",
            "purchase-token-123",
        );

        assert_ne!(first, second);
        assert!(first.starts_with("verify-purchase-"));
    }
}

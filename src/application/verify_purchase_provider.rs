use chrono::{Duration, Utc};
use serde::de::DeserializeOwned;
use std::fs;
use uuid::Uuid;

use crate::application::verify_purchase_types::{
    compute_obfuscated_id_hash, PaymentAcknowledgement, ProductType, VerificationOutcome,
    VerifyPurchaseCallback, VerifiedPurchase,
};
use crate::error::BridgeError;
use crate::ports::VerifyPurchaseRepository;
use crate::services::google_play::{
    client::GooglePlayClient,
    models::{Money, ProductPurchase, SubscriptionPurchaseV2},
};
use crate::webhooks::processor::CanonicalWebhookPayload;

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
        "creem" => verify_creem(subscription_id, purchase_token, provider_config)
            .await
            .map(VerificationOutcome::Verified),
        "lemonsqueezy" => verify_lemonsqueezy(subscription_id, purchase_token, provider_config)
            .await
            .map(VerificationOutcome::Verified),
        "coinbase" => verify_coinbase(subscription_id, purchase_token, provider_config)
            .await
            .map(VerificationOutcome::Verified),
        _ => Err(BridgeError::ValidationError(format!(
            "Unknown provider: {}",
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
    if mock_external_apis_enabled() {
        return mock_verify_google_play(subscription_id, purchase_token, product_type, external_user_id);
    }

    let client = build_google_play_client(config).await?;

    match product_type {
        ProductType::Subscription => {
            let purchase = client
                .get_subscription(google_package_name(config)?, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play verify failed: {}", e)))?;

            map_google_subscription_verification(purchase, external_user_id)
        }
        ProductType::OneTimeProduct => {
            let purchase = client
                .get_product(google_package_name(config)?, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play verify failed: {}", e)))?;

            let amount_cents = match purchase.order_id.as_deref() {
                Some(order_id) => match client
                    .get_order_amount_cents(google_package_name(config)?, order_id)
                    .await
                {
                    Ok(amount_cents) => amount_cents,
                    Err(err) => {
                        tracing::warn!(
                            "Google Play order amount lookup failed for order {}: {}",
                            order_id,
                            err
                        );
                        None
                    }
                },
                None => None,
            };

            Ok(VerificationOutcome::Verified(map_google_product_verification(
                purchase,
                amount_cents,
            )))
        }
    }
}

async fn verify_creem(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<VerifiedPurchase, BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Creem api_key".to_string()))?;
    
    let api_url = config.get("api_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.creem.com");

    let client = reqwest::Client::new();
    let url = format!(
        "{}/subscriptions/{}/verify",
        api_url.trim_end_matches('/'),
        subscription_id
    );

    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "purchase_token": purchase_token }))
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Creem verify failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "Creem verify failed: {} - {}",
            status,
            error_msg
        )));
    }

    let resp_json: serde_json::Value = response.json().await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to parse Creem response: {}", e)))?;

    let sub_status = resp_json["status"].as_str().unwrap_or("active").to_string();
    let period_end = resp_json["current_period_end"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let amount_cents = first_minor_unit_amount(&resp_json, &["/price", "/total", "/amount"]);

    tracing::info!("Creem subscription {} verified with status: {}", subscription_id, sub_status);
    Ok(VerifiedPurchase {
        status: sub_status,
        current_period_end: period_end,
        auto_renewing: None,
        amount_cents,
        payment_state: None,
        acknowledgement: PaymentAcknowledgement::NotApplicable,
        obfuscated_account_id: None,
        resubscribe_obfuscated_account_id: None,
    })
}

async fn verify_lemonsqueezy(
    subscription_id: &str,
    _purchase_token: &str,
    config: &serde_json::Value,
) -> Result<VerifiedPurchase, BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing LemonSqueezy api_key".to_string()))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.lemonsqueezy.com/v1/subscriptions/{}",
        subscription_id
    );

    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/vnd.api+json")
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy verify failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "LemonSqueezy verify failed: {} - {}",
            status,
            error_msg
        )));
    }

    let resp_json: serde_json::Value = response.json().await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to parse LemonSqueezy response: {}", e)))?;

    let sub_status = resp_json["data"]["attributes"]["status"].as_str().unwrap_or("active").to_string();
    let period_end = resp_json["data"]["attributes"]["renews_at"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let amount_cents = first_minor_unit_amount(
        &resp_json,
        &["/data/attributes/total_cents", "/data/attributes/total"],
    );

    tracing::info!("LemonSqueezy subscription {} verified with status: {}", subscription_id, sub_status);
    Ok(VerifiedPurchase {
        status: sub_status,
        current_period_end: period_end,
        auto_renewing: None,
        amount_cents,
        payment_state: None,
        acknowledgement: PaymentAcknowledgement::NotApplicable,
        obfuscated_account_id: None,
        resubscribe_obfuscated_account_id: None,
    })
}

async fn verify_coinbase(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<VerifiedPurchase, BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Coinbase api_key".to_string()))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.commerce.coinbase.com/charges/{}",
        purchase_token
    );

    let response = client
        .get(&url)
        .header("X-CC-Api-Key", api_key)
        .header("X-CC-Version", "2018-03-22")
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Coinbase verify failed: {}", e)))?;

    let status_code = response.status();
    if !status_code.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "Coinbase verify failed: {} - {}",
            status_code,
            error_msg
        )));
    }

    let resp_json: serde_json::Value = response.json().await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to parse Coinbase response: {}", e)))?;

    let charge_status = resp_json["data"]["status"].as_str().unwrap_or("completed").to_string();
    let confirmed = resp_json["data"]["confirmed_at"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let amount_cents = resp_json["data"]["payments"]
        .as_array()
        .and_then(|payments| {
            let total = payments
                .iter()
                .filter(|payment| payment["status"].as_str() == Some("CONFIRMED"))
                .filter_map(|payment| {
                    payment["value"]["local"]["amount"]
                        .as_str()
                        .and_then(|amount| amount.parse::<f64>().ok())
                })
                .map(|amount| (amount * 100.0).round() as i64)
                .sum::<i64>();

            (total > 0)
                .then_some(total)
                .and_then(|total| i32::try_from(total).ok())
        })
        .or_else(|| parse_major_unit_amount_to_cents(&resp_json["data"]["pricing"]["local"]["amount"]));

    let verified_status = if charge_status == "completed" { "active".to_string() } else { "pending".to_string() };
    
    tracing::info!("Coinbase charge {} verified with status: {}", subscription_id, verified_status);
    Ok(VerifiedPurchase {
        status: verified_status,
        current_period_end: confirmed,
        auto_renewing: None,
        amount_cents,
        payment_state: None,
        acknowledgement: PaymentAcknowledgement::NotApplicable,
        obfuscated_account_id: None,
        resubscribe_obfuscated_account_id: None,
    })
}

pub(crate) async fn forward_verify_purchase_callback<
    R: VerifyPurchaseRepository + crate::ports::AppProviderRepository + ?Sized,
>(
    repo: &R,
    app_id: uuid::Uuid,
    app_slug: &str,
    callback: VerifyPurchaseCallback<'_>,
) -> Result<(), BridgeError> {
    let now = chrono::Utc::now();
    let event_id = format!("verify-purchase-{}", Uuid::new_v4());

    let (webhook_id, _) = repo
        .create_webhook_provider(
        app_id,
        &callback.request.provider,
        &event_id,
        "verify_purchase.succeeded",
        Some(callback.request.subscription_id.clone()),
        Some(callback.request.purchase_token.clone()),
        serde_json::json!({
            "source": "verify_purchase",
            "external_user_id": callback.resolved_external_user_id,
            "subscription_id": callback.request.subscription_id,
            "provider": callback.request.provider,
            "status": callback.status,
        }),
        Some(now.timestamp_millis()),
    )
    .await?;

    let delivery_id = repo.create_webhook_delivery(app_id, webhook_id).await?;

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
        provider_event_id: event_id,
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: None,
        cancellation_mode: None,
    };

    crate::webhooks::forwarding::forward_webhook(repo, app_id, delivery_id, callback_payload).await
}

pub(crate) async fn acknowledge_google_play(
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    config: &serde_json::Value,
) -> Result<(), BridgeError> {
    if mock_external_apis_enabled() {
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

fn mock_external_apis_enabled() -> bool {
    matches!(
        std::env::var("MOCK_EXTERNAL_APIS")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
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

fn parse_minor_unit_amount(value: &serde_json::Value) -> Option<i32> {
    value
        .as_i64()
        .and_then(|amount| i32::try_from(amount).ok())
        .or_else(|| {
            value
                .as_str()
                .and_then(|amount| amount.parse::<i64>().ok())
                .and_then(|amount| i32::try_from(amount).ok())
        })
}

fn parse_major_unit_amount_to_cents(value: &serde_json::Value) -> Option<i32> {
    let amount = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|amount| amount.parse::<f64>().ok()))?;

    let cents = (amount * 100.0).round();
    if !cents.is_finite() || cents < 0.0 || cents > i32::MAX as f64 {
        return None;
    }

    Some(cents as i32)
}

fn first_minor_unit_amount(resp_json: &serde_json::Value, paths: &[&str]) -> Option<i32> {
    paths
        .iter()
        .find_map(|path| resp_json.pointer(path).and_then(parse_minor_unit_amount))
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

    let amount_cents = purchase
        .line_items
        .iter()
        .find_map(|line_item| line_item.auto_renewing_plan.as_ref())
        .and_then(|plan| plan.recurring_price.as_ref())
        .and_then(google_money_to_cents);

    let acknowledgement = match purchase.acknowledgement_state.as_deref() {
        Some("ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED") => PaymentAcknowledgement::AlreadyAcknowledged,
        _ => PaymentAcknowledgement::Pending,
    };

    Ok(VerificationOutcome::Verified(VerifiedPurchase {
        status,
        current_period_end,
        auto_renewing,
        amount_cents,
        payment_state: None,
        acknowledgement,
        obfuscated_account_id,
        resubscribe_obfuscated_account_id,
    }))
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
        current_period_end: None,
        auto_renewing: None,
        amount_cents,
        payment_state: Some(purchase.purchase_state),
        acknowledgement,
        obfuscated_account_id: purchase.obfuscated_account_id,
        resubscribe_obfuscated_account_id: None,
    }
}

fn mock_verify_google_play(
    _subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
) -> Result<VerificationOutcome, BridgeError> {
    match product_type {
        ProductType::Subscription => {
            if let Some(purchase) = load_mock_fixture::<SubscriptionPurchaseV2>(
                "MOCK_GOOGLE_PURCHASE_RESPONSE",
            ) {
                return map_google_subscription_verification(purchase, external_user_id);
            }

            if purchase_token == "resubscribe-linking-required" {
                return Ok(VerificationOutcome::LinkingRequired {
                    obfuscated_account_id: "sub-19b-owner-hash".to_string(),
                });
            }

            if purchase_token.contains("oap") || purchase_token.contains("resubscribe") {
                let obfuscated_account_id = compute_obfuscated_id_hash(external_user_id);

                return Ok(VerificationOutcome::Verified(VerifiedPurchase {
                    status: "active".to_string(),
                    current_period_end: Some(Utc::now() + Duration::days(30)),
                    auto_renewing: Some(true),
                    amount_cents: None,
                    payment_state: None,
                    acknowledgement: PaymentAcknowledgement::Pending,
                    obfuscated_account_id: Some(obfuscated_account_id.clone()),
                    resubscribe_obfuscated_account_id: Some(obfuscated_account_id),
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

            Ok(VerificationOutcome::Verified(VerifiedPurchase {
                status,
                current_period_end: Some(Utc::now() + Duration::days(30)),
                auto_renewing: Some(
                    !purchase_token.contains("cancelled") && !purchase_token.contains("canceled"),
                ),
                amount_cents: None,
                payment_state: None,
                acknowledgement: PaymentAcknowledgement::Pending,
                obfuscated_account_id: Some(compute_obfuscated_id_hash(external_user_id)),
                resubscribe_obfuscated_account_id: None,
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
                current_period_end: None,
                auto_renewing: None,
                amount_cents: None,
                payment_state: Some(if purchase_token.contains("slow") || purchase_token.contains("pending") {
                    2
                } else {
                    0
                }),
                acknowledgement: PaymentAcknowledgement::Pending,
                obfuscated_account_id: None,
                resubscribe_obfuscated_account_id: None,
            }))
        }
    }
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
        AutoRenewingPlan, ExternalAccountIdentifiers, OutOfAppPurchaseContext, SubscriptionLineItem,
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
                auto_renewing_plan: Some(AutoRenewingPlan {
                    auto_renew_enabled: Some(true),
                    recurring_price: Some(Money {
                        currency_code: Some("USD".to_string()),
                        units: Some("12".to_string()),
                        nanos: Some(990_000_000),
                    }),
                }),
                offer_details: None,
            }],
            ..Default::default()
        };

        let verification = map_google_subscription_verification(purchase, "user-123")
            .expect("google subscription verification should succeed");

        match verification {
            VerificationOutcome::Verified(verified) => {
                assert_eq!(verified.amount_cents, Some(1299));
                assert_eq!(verified.status, "active");
            }
            VerificationOutcome::LinkingRequired { .. } => {
                panic!("expected a verified purchase response")
            }
        }
    }

    #[test]
    fn google_subscription_verification_prefers_expired_obfuscated_account_id_for_resubscribe() {
        let purchase = SubscriptionPurchaseV2 {
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
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

        let verification = map_google_subscription_verification(purchase, "user-123")
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

        let verification = map_google_subscription_verification(purchase, "user-123")
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

        let verified = map_google_product_verification(purchase, Some(499));

        assert_eq!(verified.amount_cents, Some(499));
        assert_eq!(verified.status, "active");
        assert_eq!(verified.payment_state, Some(0));
    }
}

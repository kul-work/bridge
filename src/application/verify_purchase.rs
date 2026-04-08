use chrono::Utc;
use uuid::Uuid;

use crate::error::BridgeError;
use crate::ports::{
    AppLookupRepository, GooglePlayAccountLookupRepository, PaymentAcknowledgementRepository,
    ProviderConfigLookupRepository, SubscriptionLookupRepository, VerifyPurchaseRepository,
    WebhookForwardRepository, WebhookWriteRepository,
};
use crate::application::verify_purchase_types::{
    compute_obfuscated_id_hash, PaymentAcknowledgement, ProductType, VerificationOutcome,
    VerifyPurchaseCallback, VerifyPurchaseCommitRequest, VerifyPurchaseRequest,
    VerifyPurchaseResponse,
};
use crate::application::verify_purchase_provider::{
    acknowledge_google_play, forward_verify_purchase_callback, verify_purchase_with_provider,
};

pub async fn verify_purchase<
    R: AppLookupRepository
        + GooglePlayAccountLookupRepository
        + PaymentAcknowledgementRepository
        + ProviderConfigLookupRepository
        + SubscriptionLookupRepository
        + VerifyPurchaseRepository
        + WebhookForwardRepository
        + WebhookWriteRepository
        + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    payload: VerifyPurchaseRequest,
) -> Result<VerifyPurchaseResponse, BridgeError> {
    if payload.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }
    if payload.provider.is_empty() {
        return Err(BridgeError::ValidationError(
            "provider is required".to_string(),
        ));
    }
    if payload.subscription_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "subscription_id is required".to_string(),
        ));
    }
    if payload.purchase_token.is_empty() {
        return Err(BridgeError::ValidationError(
            "purchase_token is required".to_string(),
        ));
    }
    if payload.product_type.trim().is_empty() {
        return Err(BridgeError::ValidationError(
            "product_type is required".to_string(),
        ));
    }

    let product_type = ProductType::parse(&payload.product_type)?;

    let app = repo.get_app(app_id).await?;
    let provider_config = repo.get_provider_config(app_id, &payload.provider).await?;

    let verification = verify_purchase_with_provider(
        &payload.provider,
        &payload.subscription_id,
        &payload.purchase_token,
        product_type,
        &payload.external_user_id,
        &provider_config.config,
    )
    .await?;

    let verified = match verification {
        VerificationOutcome::LinkingRequired {
            obfuscated_account_id,
        } => {
            return Ok(VerifyPurchaseResponse {
                    status: "linking_required".to_string(),
                    subscription_id: payload.subscription_id.clone(),
                    current_period_end: None,
                    auto_renewing: None,
                    amount_cents: None,
                    is_new: false,
                    message: Some(
                        "This purchase belongs to a different Google Play account and must be linked first"
                            .to_string(),
                    ),
                    obfuscated_account_id: Some(obfuscated_account_id),
                });
        }
        VerificationOutcome::Verified(verified) => verified,
    };

    let mut resolved_external_user_id = payload.external_user_id.clone();

    if payload.provider == "google_play" {
        if let Some(resubscribe_obfuscated_account_id) =
            verified.resubscribe_obfuscated_account_id.as_deref()
        {
            resolved_external_user_id = match repo
                .lookup_user_by_google_obfuscated_id(app_id, resubscribe_obfuscated_account_id)
                .await?
            {
                Some(original_external_user_id) => original_external_user_id,
                None => {
                    return Ok(VerifyPurchaseResponse {
                            status: "linking_required".to_string(),
                            subscription_id: payload.subscription_id.clone(),
                            current_period_end: None,
                            auto_renewing: None,
                            amount_cents: verified.amount_cents,
                            is_new: false,
                            message: Some(
                                "This purchase belongs to a different Google Play account and must be linked first"
                                    .to_string(),
                            ),
                            obfuscated_account_id: Some(
                                resubscribe_obfuscated_account_id.to_string(),
                            ),
                        });
                }
            };
        } else if let Some(owner_hash) = verified.obfuscated_account_id.as_deref() {
            if compute_obfuscated_id_hash(&payload.external_user_id) != owner_hash {
                return Ok(VerifyPurchaseResponse {
                        status: "linking_required".to_string(),
                        subscription_id: payload.subscription_id.clone(),
                        current_period_end: None,
                        auto_renewing: None,
                        amount_cents: verified.amount_cents,
                        is_new: false,
                        message: Some(
                            "This purchase belongs to a different Google Play account and must be linked first"
                                .to_string(),
                        ),
                        obfuscated_account_id: Some(owner_hash.to_string()),
                    });
            }
        }
    }

    let existing_subscription = if product_type.is_subscription() {
        repo.get_subscription(
            app_id,
            &resolved_external_user_id,
            &payload.subscription_id,
            &payload.provider,
        )
        .await?
    } else {
        None
    };

    let is_new = existing_subscription.is_none();

    if product_type.is_subscription() {
        if let Some(token_subscription) = repo
            .get_subscription_by_purchase_token(app_id, &payload.purchase_token)
            .await?
        {
            if token_subscription.external_user_id != payload.external_user_id {
                if payload.provider == "google_play" {
                    if let Some(obfuscated_account_id) = verified.obfuscated_account_id.clone() {
                        return Ok(VerifyPurchaseResponse {
                            status: "linking_required".to_string(),
                            subscription_id: payload.subscription_id.clone(),
                            current_period_end: None,
                            auto_renewing: None,
                            amount_cents: verified.amount_cents,
                            is_new: false,
                            message: Some(
                                "This purchase token is already owned by a different user and must be linked first"
                                    .to_string(),
                            ),
                            obfuscated_account_id: Some(obfuscated_account_id),
                        });
                    }
                }

                return Err(BridgeError::FraudDetected(
                    "Purchase token already bound to different user".to_string(),
                ));
            }

            if token_subscription.subscription_id != payload.subscription_id
                || token_subscription.provider != payload.provider
            {
                return Err(BridgeError::ValidationError(
                    "Purchase token already linked to a different subscription".to_string(),
                ));
            }
        }
    }

    let current_period_end = verified.current_period_end.or_else(|| {
        existing_subscription
            .as_ref()
            .and_then(|subscription| subscription.current_period_end)
    });
    let response_current_period_end = current_period_end.as_ref().map(|d| d.to_rfc3339());
    let payment_status = product_type.payment_status(&verified.status).to_string();

    let commit_result = match repo
        .commit_verified_purchase(VerifyPurchaseCommitRequest {
            app_id,
            resolved_external_user_id: &resolved_external_user_id,
            provider: &payload.provider,
            subscription_id: &payload.subscription_id,
            purchase_token: &payload.purchase_token,
            subscription_status: &verified.status,
            payment_status: &payment_status,
            current_period_end,
            auto_renewing: verified.auto_renewing.or_else(|| {
                existing_subscription
                    .as_ref()
                    .and_then(|subscription| subscription.auto_renewing)
            }),
            payment_state: verified.payment_state.or_else(|| {
                existing_subscription
                    .as_ref()
                    .and_then(|subscription| subscription.payment_state)
            }),
            provider_customer_id: existing_subscription
                .as_ref()
                .and_then(|subscription| subscription.provider_customer_id.as_deref()),
            google_obfuscated_account_id: verified.obfuscated_account_id.as_deref(),
            amount_cents: verified.amount_cents.unwrap_or(0),
            event_time_ms: Utc::now().timestamp_millis(),
            is_subscription: product_type.is_subscription(),
        })
        .await
    {
        Ok(result) => result,
        Err(err) => {
            if payload.provider == "google_play" {
                if let BridgeError::FraudDetected(_) = &err {
                    if let Some(obfuscated_account_id) = verified.obfuscated_account_id.clone() {
                        return Ok(VerifyPurchaseResponse {
                            status: "linking_required".to_string(),
                            subscription_id: payload.subscription_id.clone(),
                            current_period_end: None,
                            auto_renewing: None,
                            amount_cents: verified.amount_cents,
                            is_new: false,
                            message: Some(
                                "This purchase belongs to a different Google Play account and must be linked first"
                                    .to_string(),
                            ),
                            obfuscated_account_id: Some(obfuscated_account_id),
                        });
                    }
                }
            }

            return Err(err);
        }
    };

    let mut response_auto_renewing = verified.auto_renewing;

    if let Some(subscription) = commit_result.subscription {
        response_auto_renewing = subscription.auto_renewing;
    }

    let payment_acknowledged = repo
        .payment_acknowledged_at(app_id, &payload.provider, &payload.purchase_token)
        .await?
        .is_some();

    match verified.acknowledgement {
        PaymentAcknowledgement::AlreadyAcknowledged => {
            repo.mark_payment_acknowledged(app_id, &payload.provider, &payload.purchase_token)
                .await?;
        }
        PaymentAcknowledgement::Pending
            if payload.provider == "google_play" && !payment_acknowledged =>
        {
            if let Err(err) = acknowledge_google_play(
                &payload.subscription_id,
                &payload.purchase_token,
                product_type,
                &provider_config.config,
            )
            .await
            {
                tracing::warn!(
                    "verify_purchase acknowledgement failed for app {} token {}: {}",
                    app.id,
                    payload.purchase_token,
                    err
                );
            } else {
                repo.mark_payment_acknowledged(app_id, &payload.provider, &payload.purchase_token)
                    .await?;
            }
        }
        PaymentAcknowledgement::NotApplicable | PaymentAcknowledgement::Pending => {}
    }

    let response = VerifyPurchaseResponse {
        status: verified.status.clone(),
        subscription_id: payload.subscription_id.clone(),
        current_period_end: response_current_period_end,
        auto_renewing: response_auto_renewing,
        amount_cents: verified.amount_cents,
        is_new,
        message: None,
        obfuscated_account_id: None,
    };

    if response.status != "pending" {
        let callback_status = product_type.callback_status(&response.status);

        if let Err(e) = forward_verify_purchase_callback(
            repo,
            app_id,
            &app.slug,
            VerifyPurchaseCallback {
                request: &payload,
                resolved_external_user_id: &resolved_external_user_id,
                product_type,
                status: &callback_status,
                current_period_end: response.current_period_end.as_deref(),
                auto_renewing: response.auto_renewing,
                amount_cents: response.amount_cents,
            },
        )
        .await
        {
            tracing::warn!(
                "verify_purchase callback forwarding failed for app {} sub {}: {}",
                app.id,
                payload.subscription_id,
                e
            );
        }
    }

    Ok(response)
}

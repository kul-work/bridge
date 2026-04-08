use uuid::Uuid;

use crate::application::subscription_actions_types::{
    BillingPortalResponse, CancelSubscriptionRequest, CancelSubscriptionResponse,
    PriceStepUpAcceptResponse, PriceStepUpDeclineResponse, PriceStepUpRequest,
    ResumeSubscriptionResponse, SubscriptionActionQuery, SubscriptionActionResponse,
};
use crate::db;
use crate::error::BridgeError;
use crate::ports::{
    AppWebhookRepository, SubscriptionReadRepository, SubscriptionWriteRepository,
};
use crate::services::provider_api;

pub async fn cancel_subscription<R, S, W>(
    app_repo: &R,
    subscription_repo: &S,
    subscription_write_repo: &W,
    app_id: Uuid,
    subscription_id: &str,
    query: SubscriptionActionQuery,
    request: Option<CancelSubscriptionRequest>,
) -> Result<CancelSubscriptionResponse, BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
    S: SubscriptionReadRepository + Send + Sync + ?Sized,
    W: SubscriptionWriteRepository + Send + Sync + ?Sized,
{
    if query.external_user_id.trim().is_empty() {
        return Err(BridgeError::ValidationError("external_user_id is required".to_string()));
    }

    if query.provider.trim().is_empty() {
        return Err(BridgeError::ValidationError("provider is required".to_string()));
    }

    let request = request.unwrap_or_default();
    let provider = query.provider.trim().to_ascii_lowercase();
    let mode = request.mode.as_deref().unwrap_or("scheduled");

    let sub = subscription_repo
        .get_subscription(
            app_id,
            query.external_user_id.trim(),
            subscription_id,
            &provider,
        )
        .await?;

    let purchase_token = request
        .purchase_token
        .as_deref()
        .or(sub.purchase_token.as_deref());

    if sub.provider == "google_play" && purchase_token.is_none() {
        return Err(BridgeError::SubscriptionNotFound(
            "Google Play purchase token not found for this subscription".to_string(),
        ));
    }

    let provider_config = app_repo.get_provider_config(app_id, &sub.provider).await?;

    provider_api::cancel_subscription(
        &sub.provider,
        &sub.subscription_id,
        purchase_token,
        Some(mode),
        &provider_config.config,
    )
    .await?;

    let updated_sub = match mode {
        "scheduled" => subscription_write_repo.cancel_subscription_scheduled(sub.id).await?,
        "immediate" => subscription_write_repo.cancel_subscription_immediate(sub.id).await?,
        _ => {
            return Err(BridgeError::ValidationError(
                "mode must be either 'scheduled' or 'immediate'".to_string(),
            ))
        }
    };

    if let Err(e) = dispatch_subscription_callback(
        app_repo,
        app_id,
        &updated_sub,
        "subscription.cancelled",
        Some(mode),
        None,
    )
    .await
    {
        tracing::warn!(
            "Failed to forward cancel callback for {}: {}",
            updated_sub.subscription_id,
            e
        );
    }

    Ok(CancelSubscriptionResponse {
        status: "cancelled".to_string(),
        mode: mode.to_string(),
        subscription_id: updated_sub.subscription_id,
    })
}

pub async fn resume_subscription<R, S, W>(
    app_repo: &R,
    subscription_repo: &S,
    subscription_write_repo: &W,
    app_id: Uuid,
    subscription_id: &str,
    query: SubscriptionActionQuery,
) -> Result<ResumeSubscriptionResponse, BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
    S: SubscriptionReadRepository + Send + Sync + ?Sized,
    W: SubscriptionWriteRepository + Send + Sync + ?Sized,
{
    if query.external_user_id.trim().is_empty() {
        return Err(BridgeError::ValidationError("external_user_id is required".to_string()));
    }

    if query.provider.trim().is_empty() {
        return Err(BridgeError::ValidationError("provider is required".to_string()));
    }

    let provider = query.provider.trim().to_ascii_lowercase();
    let sub = subscription_repo
        .get_subscription(
            app_id,
            query.external_user_id.trim(),
            subscription_id,
            &provider,
        )
        .await?;

    let provider_config = app_repo.get_provider_config(app_id, &sub.provider).await?;

    provider_api::resume_subscription(
        &sub.provider,
        &sub.subscription_id,
        &provider_config.config,
    )
    .await?;

    let updated_sub = subscription_write_repo.resume_subscription(sub.id).await?;

    if let Err(e) = dispatch_subscription_callback(
        app_repo,
        app_id,
        &updated_sub,
        "subscription.resumed",
        None,
        None,
    )
    .await
    {
        tracing::warn!(
            "Failed to forward resume callback for {}: {}",
            updated_sub.subscription_id,
            e
        );
    }

    Ok(ResumeSubscriptionResponse {
        status: updated_sub.status,
        subscription_id: updated_sub.subscription_id,
    })
}

pub async fn acknowledge_subscription<R, W>(
    app_repo: &R,
    subscription_write_repo: &W,
    app_id: Uuid,
    subscription_id: &str,
    external_user_id: &str,
) -> Result<SubscriptionActionResponse, BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
    W: SubscriptionWriteRepository + Send + Sync + ?Sized,
{
    let sub = app_repo
        .get_subscription_by_sub_id(app_id, subscription_id)
        .await?
        .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != external_user_id {
        return Err(BridgeError::ValidationError(
            "Subscription does not belong to this user".to_string(),
        ));
    }

    subscription_write_repo
        .mark_payment_acknowledged_for_subscription(
            app_id,
            &sub.external_user_id,
            &sub.provider,
            &sub.subscription_id,
            sub.purchase_token.as_deref(),
        )
        .await?;

    Ok(SubscriptionActionResponse {
        success: true,
        message: "Subscription acknowledged".to_string(),
    })
}

pub async fn create_billing_portal<R, S>(
    app_repo: &R,
    subscription_repo: &S,
    app_id: Uuid,
    subscription_id: &str,
    query: SubscriptionActionQuery,
) -> Result<BillingPortalResponse, BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
    S: SubscriptionReadRepository + Send + Sync + ?Sized,
{
    if query.external_user_id.trim().is_empty() {
        return Err(BridgeError::ValidationError("external_user_id is required".to_string()));
    }

    if query.provider.trim().is_empty() {
        return Err(BridgeError::ValidationError("provider is required".to_string()));
    }

    let provider = query.provider.trim().to_ascii_lowercase();
    let sub = subscription_repo
        .get_subscription(
            app_id,
            query.external_user_id.trim(),
            subscription_id,
            &provider,
        )
        .await?;

    let customer_id = sub.provider_customer_id.as_deref().ok_or_else(|| {
        BridgeError::ValidationError(
            "Provider customer ID not available for this subscription".to_string(),
        )
    })?;

    let provider_config = app_repo.get_provider_config(app_id, &sub.provider).await?;

    let url = provider_api::create_billing_portal(
        &sub.provider,
        customer_id,
        &provider_config.config,
    )
    .await?;

    Ok(BillingPortalResponse { url })
}

pub async fn accept_price_step_up<R, W>(
    app_repo: &R,
    subscription_write_repo: &W,
    app_id: Uuid,
    subscription_id: &str,
    request: PriceStepUpRequest,
) -> Result<PriceStepUpAcceptResponse, BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
    W: SubscriptionWriteRepository + Send + Sync + ?Sized,
{
    let sub = app_repo
        .get_subscription_by_sub_id(app_id, subscription_id)
        .await?
        .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError(
            "Subscription does not belong to this user".to_string(),
        ));
    }

    if sub.provider != "google_play" {
        return Err(BridgeError::ValidationError(
            "Price step-up actions are only supported for Google Play subscriptions".to_string(),
        ));
    }

    let updated_sub = subscription_write_repo.accept_price_step_up(sub.id).await?;

    let new_price_cents = updated_sub.google_new_price_cents.ok_or_else(|| {
        BridgeError::ValidationError(
            "Google Play price step-up amount not available for this subscription".to_string(),
        )
    })?;

    if let Err(e) = dispatch_subscription_callback(
        app_repo,
        app_id,
        &updated_sub,
        "subscription.price_step_up",
        None,
        Some(new_price_cents),
    )
    .await
    {
        tracing::warn!(
            "Failed to forward price step-up accept callback for {}: {}",
            updated_sub.subscription_id,
            e
        );
    }

    Ok(PriceStepUpAcceptResponse {
        accepted: true,
        new_price_cents,
    })
}

pub async fn decline_price_step_up<R, W>(
    _app_repo: &R,
    subscription_write_repo: &W,
    app_id: Uuid,
    subscription_id: &str,
    request: PriceStepUpRequest,
) -> Result<PriceStepUpDeclineResponse, BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
    W: SubscriptionWriteRepository + Send + Sync + ?Sized,
{
    let sub = _app_repo
        .get_subscription_by_sub_id(app_id, subscription_id)
        .await?
        .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError(
            "Subscription does not belong to this user".to_string(),
        ));
    }

    if sub.provider != "google_play" {
        return Err(BridgeError::ValidationError(
            "Price step-up actions are only supported for Google Play subscriptions".to_string(),
        ));
    }

    let updated_sub = subscription_write_repo.decline_price_step_up(sub.id).await?;

    let cancellation_effective_at = updated_sub
        .current_period_end
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    Ok(PriceStepUpDeclineResponse {
        declined: true,
        cancellation_effective_at,
    })
}

async fn dispatch_subscription_callback<R>(
    repo: &R,
    app_id: Uuid,
    sub: &db::subscriptions::Subscription,
    event_type: &str,
    cancellation_mode: Option<&str>,
    new_price_cents: Option<i32>,
) -> Result<(), BridgeError>
where
    R: AppWebhookRepository + Send + Sync + ?Sized,
{
    let app = repo.get_app(app_id).await?;
    let provider_event_id = format!("manual-{}", Uuid::new_v4());
    let timestamp_epoch_ms = chrono::Utc::now().timestamp_millis();
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    let payload = serde_json::json!({
        "source": "api",
        "event_type": event_type,
        "subscription_id": sub.subscription_id,
        "external_user_id": sub.external_user_id,
        "provider": sub.provider,
        "mode": cancellation_mode,
        "new_price_cents": new_price_cents,
    });

    let (webhook_provider_id, _) = repo
        .create_webhook_provider(
            app_id,
            &sub.provider,
            &provider_event_id,
            event_type,
            Some(sub.subscription_id.clone()),
            sub.purchase_token.clone(),
            payload,
            Some(timestamp_epoch_ms),
        )
        .await?;

    let delivery_id = repo.create_webhook_delivery(app_id, webhook_provider_id).await?;

    let canonical = crate::webhooks::processor::CanonicalWebhookPayload {
        event_id: format!("{}-{}", sub.provider, provider_event_id),
        event_type: event_type.to_string(),
        timestamp,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: None,
        subscription_id: Some(sub.subscription_id.clone()),
        external_user_id: Some(sub.external_user_id.clone()),
        amount_cents: None,
        new_price_cents,
        auto_renewing: sub.auto_renewing,
        purchase_token: sub.purchase_token.clone(),
        current_period_end: sub.current_period_end.map(|d| d.to_rfc3339()),
        status: Some(sub.status.clone()),
        provider: sub.provider.clone(),
        provider_event_id,
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: sub.revocation_reason.clone(),
        cancellation_mode: cancellation_mode.map(str::to_string),
    };

    crate::webhooks::forwarding::forward_webhook(repo, app_id, delivery_id, canonical).await
}

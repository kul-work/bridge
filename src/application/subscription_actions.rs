use uuid::Uuid;

use crate::application::subscription_actions_types::{
    BillingPortalResponse, CancelSubscriptionRequest, CancelSubscriptionResponse,
    PriceStepUpAcceptResponse, PriceStepUpDeclineResponse, PriceStepUpRequest,
    ResumeSubscriptionResponse, SubscriptionActionQuery, SubscriptionActionResponse,
};
use crate::error::BridgeError;
use crate::ports::SubscriptionActionsHandlerRepository;
use crate::services::provider_api;
use crate::utils::diagnostic_hash;

pub(crate) struct CancelSubscriptionInput {
    pub app_id: Uuid,
    pub subscription_id: String,
    pub query: SubscriptionActionQuery,
    pub request: Option<CancelSubscriptionRequest>,
}

pub async fn cancel_subscription<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    input: CancelSubscriptionInput,
) -> Result<CancelSubscriptionResponse, BridgeError>
{
    let CancelSubscriptionInput {
        app_id,
        subscription_id,
        query,
        request,
    } = input;

    if query.external_user_id.trim().is_empty() {
        return Err(BridgeError::ValidationError("external_user_id is required".to_string()));
    }

    if query.provider.trim().is_empty() {
        return Err(BridgeError::ValidationError("provider is required".to_string()));
    }

    let request = request.unwrap_or_default();
    let provider = query.provider.trim().to_ascii_lowercase();
    let mode = request.mode.as_deref().unwrap_or("scheduled");

    if mode != "scheduled" && mode != "immediate" {
        return Err(BridgeError::ValidationError(
            "mode must be either 'scheduled' or 'immediate'".to_string(),
        ));
    }

    let sub = repo
        .get_subscription(
            app_id,
            query.external_user_id.trim(),
            &subscription_id,
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

    let provider_config = repo.get_provider_config(app_id, &sub.provider).await?;

    let on_execute = request.on_execute.as_deref();

    provider_api::cancel_subscription(
        &sub.provider,
        &sub.subscription_id,
        purchase_token,
        Some(mode),
        on_execute,
        &provider_config.config,
    )
    .await?;

    // mode already validated above; exhaustive match kept for safety
    let updated_sub = match mode {
        "scheduled" => repo.cancel_subscription_scheduled(app_id, sub.id).await?,
        "immediate" => repo.cancel_subscription_immediate(app_id, sub.id).await?,
        _ => unreachable!("mode validated earlier"),
    };

    let callback_sub = SubscriptionCallbackData {
        subscription_id: updated_sub.subscription_id.clone(),
        external_user_id: updated_sub.external_user_id.clone(),
        provider: updated_sub.provider.clone(),
        purchase_token: updated_sub.purchase_token.clone(),
        auto_renewing: updated_sub.auto_renewing,
        current_period_end: updated_sub.current_period_end,
        status: updated_sub.status.clone(),
        revocation_reason: updated_sub.revocation_reason.clone(),
        google_price_step_up_consent_deadline: updated_sub.google_price_step_up_consent_deadline,
        google_pause_scheduled_at: updated_sub.google_pause_scheduled_at,
        google_deferred_until: updated_sub.google_deferred_until,
    };

    if let Err(e) = dispatch_subscription_callback(
        repo,
        app_id,
        &callback_sub,
        "subscription.cancelled",
        Some(mode),
        None,
    )
    .await
    {
        tracing::warn!(
            operation = "cancel_subscription",
            app_id = %app_id,
            provider = %updated_sub.provider,
            external_user_id_hash = %diagnostic_hash(&updated_sub.external_user_id),
            subscription_id = %updated_sub.subscription_id,
            event_type = "subscription.cancelled",
            mode,
            error = %e,
            "Failed to forward subscription action callback"
        );
    }

    Ok(CancelSubscriptionResponse {
        status: "cancelled".to_string(),
        mode: mode.to_string(),
        subscription_id: updated_sub.subscription_id,
    })
}

pub async fn resume_subscription<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    subscription_id: &str,
    query: SubscriptionActionQuery,
) -> Result<ResumeSubscriptionResponse, BridgeError>
{
    if query.external_user_id.trim().is_empty() {
        return Err(BridgeError::ValidationError("external_user_id is required".to_string()));
    }

    if query.provider.trim().is_empty() {
        return Err(BridgeError::ValidationError("provider is required".to_string()));
    }

    let provider = query.provider.trim().to_ascii_lowercase();
    let sub = repo
        .get_subscription(
            app_id,
            query.external_user_id.trim(),
            subscription_id,
            &provider,
        )
        .await?;

    let provider_config = repo.get_provider_config(app_id, &sub.provider).await?;

    provider_api::resume_subscription(
        &sub.provider,
        &sub.subscription_id,
        &provider_config.config,
    )
    .await?;

    let updated_sub = repo.resume_subscription(app_id, sub.id).await?;

    let callback_sub = SubscriptionCallbackData {
        subscription_id: updated_sub.subscription_id.clone(),
        external_user_id: updated_sub.external_user_id.clone(),
        provider: updated_sub.provider.clone(),
        purchase_token: updated_sub.purchase_token.clone(),
        auto_renewing: updated_sub.auto_renewing,
        current_period_end: updated_sub.current_period_end,
        status: updated_sub.status.clone(),
        revocation_reason: updated_sub.revocation_reason.clone(),
        google_price_step_up_consent_deadline: updated_sub.google_price_step_up_consent_deadline,
        google_pause_scheduled_at: updated_sub.google_pause_scheduled_at,
        google_deferred_until: updated_sub.google_deferred_until,
    };

    if let Err(e) = dispatch_subscription_callback(
        repo,
        app_id,
        &callback_sub,
        "subscription.resumed",
        None,
        None,
    )
    .await
    {
        tracing::warn!(
            operation = "resume_subscription",
            app_id = %app_id,
            provider = %updated_sub.provider,
            external_user_id_hash = %diagnostic_hash(&updated_sub.external_user_id),
            subscription_id = %updated_sub.subscription_id,
            event_type = "subscription.resumed",
            error = %e,
            "Failed to forward subscription action callback"
        );
    }

    Ok(ResumeSubscriptionResponse {
        status: updated_sub.status,
        subscription_id: updated_sub.subscription_id,
    })
}

pub async fn acknowledge_subscription<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    subscription_id: &str,
    external_user_id: &str,
) -> Result<SubscriptionActionResponse, BridgeError>
{
    let sub = repo
        .get_subscription_by_sub_id_and_user(app_id, subscription_id, external_user_id)
        .await?
        .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    repo
        .mark_payment_acknowledged_for_subscription(
            app_id,
            &sub.external_user_id,
            &sub.provider,
            &sub.subscription_id,
            sub.purchase_token.as_deref(),
        )
        .await?;

    // Clear payment failure notification flag when acknowledging
    repo.clear_payment_failure_notification(
        app_id,
        &sub.external_user_id,
        &sub.provider,
        &sub.subscription_id,
    )
        .await?;

    Ok(SubscriptionActionResponse {
        success: true,
        message: "Subscription acknowledged".to_string(),
    })
}

pub async fn create_billing_portal<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    subscription_id: &str,
    query: SubscriptionActionQuery,
) -> Result<BillingPortalResponse, BridgeError>
{
    if query.external_user_id.trim().is_empty() {
        return Err(BridgeError::ValidationError("external_user_id is required".to_string()));
    }

    if query.provider.trim().is_empty() {
        return Err(BridgeError::ValidationError("provider is required".to_string()));
    }

    let provider = query.provider.trim().to_ascii_lowercase();
    let sub = repo
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

    let provider_config = repo.get_provider_config(app_id, &sub.provider).await?;

    let url = provider_api::create_billing_portal(
        &sub.provider,
        customer_id,
        &provider_config.config,
    )
    .await?;

    Ok(BillingPortalResponse { url })
}

pub async fn accept_price_step_up<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    subscription_id: &str,
    request: PriceStepUpRequest,
) -> Result<PriceStepUpAcceptResponse, BridgeError>
{
    let sub = repo
        .get_subscription_by_sub_id_and_user(app_id, subscription_id, &request.external_user_id)
        .await?
        .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.provider != "google_play" {
        return Err(BridgeError::ValidationError(
            "Price step-up actions are only supported for Google Play subscriptions".to_string(),
        ));
    }

    let updated_sub = repo.accept_price_step_up(app_id, sub.id).await?;

    let new_price_cents = updated_sub.google_new_price_cents.ok_or_else(|| {
        BridgeError::ValidationError(
            "Google Play price step-up amount not available for this subscription".to_string(),
        )
    })?;

    let callback_sub = SubscriptionCallbackData {
        subscription_id: updated_sub.subscription_id.clone(),
        external_user_id: updated_sub.external_user_id.clone(),
        provider: updated_sub.provider.clone(),
        purchase_token: updated_sub.purchase_token.clone(),
        auto_renewing: updated_sub.auto_renewing,
        current_period_end: updated_sub.current_period_end,
        status: updated_sub.status.clone(),
        revocation_reason: updated_sub.revocation_reason.clone(),
        google_price_step_up_consent_deadline: updated_sub.google_price_step_up_consent_deadline,
        google_pause_scheduled_at: updated_sub.google_pause_scheduled_at,
        google_deferred_until: updated_sub.google_deferred_until,
    };

    if let Err(e) = dispatch_subscription_callback(
        repo,
        app_id,
        &callback_sub,
        "subscription.price_step_up",
        None,
        Some(new_price_cents),
    )
    .await
    {
        tracing::warn!(
            operation = "accept_price_step_up",
            app_id = %app_id,
            provider = %updated_sub.provider,
            external_user_id_hash = %diagnostic_hash(&updated_sub.external_user_id),
            subscription_id = %updated_sub.subscription_id,
            event_type = "subscription.price_step_up",
            new_price_cents,
            error = %e,
            "Failed to forward subscription action callback"
        );
    }

    Ok(PriceStepUpAcceptResponse {
        accepted: true,
        new_price_cents,
    })
}

pub async fn decline_price_step_up<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    subscription_id: &str,
    request: PriceStepUpRequest,
) -> Result<PriceStepUpDeclineResponse, BridgeError>
{
    let sub = repo
        .get_subscription_by_sub_id_and_user(app_id, subscription_id, &request.external_user_id)
        .await?
        .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.provider != "google_play" {
        return Err(BridgeError::ValidationError(
            "Price step-up actions are only supported for Google Play subscriptions".to_string(),
        ));
    }

    let purchase_token = sub.purchase_token.as_deref().ok_or_else(|| {
        BridgeError::SubscriptionNotFound(
            "Google Play purchase token not found for this subscription".to_string(),
        )
    })?;

    let provider_config = repo.get_provider_config(app_id, &sub.provider).await?;

    provider_api::cancel_subscription(
        &sub.provider,
        &sub.subscription_id,
        Some(purchase_token),
        Some("scheduled"),
        None,
        &provider_config.config,
    )
    .await?;

    let updated_sub = repo.decline_price_step_up(app_id, sub.id).await?;

    let cancellation_effective_at = updated_sub
        .current_period_end
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let callback_sub = SubscriptionCallbackData {
        subscription_id: updated_sub.subscription_id.clone(),
        external_user_id: updated_sub.external_user_id.clone(),
        provider: updated_sub.provider.clone(),
        purchase_token: updated_sub.purchase_token.clone(),
        auto_renewing: updated_sub.auto_renewing,
        current_period_end: updated_sub.current_period_end,
        status: updated_sub.status.clone(),
        revocation_reason: updated_sub.revocation_reason.clone(),
        google_price_step_up_consent_deadline: updated_sub.google_price_step_up_consent_deadline,
        google_pause_scheduled_at: updated_sub.google_pause_scheduled_at,
        google_deferred_until: updated_sub.google_deferred_until,
    };

    if let Err(e) = dispatch_subscription_callback(
        repo,
        app_id,
        &callback_sub,
        "subscription.cancelled",
        Some("scheduled"),
        None,
    )
    .await
    {
        tracing::warn!(
            operation = "decline_price_step_up",
            app_id = %app_id,
            provider = %updated_sub.provider,
            external_user_id_hash = %diagnostic_hash(&updated_sub.external_user_id),
            subscription_id = %updated_sub.subscription_id,
            event_type = "subscription.cancelled",
            mode = "scheduled",
            error = %e,
            "Failed to forward subscription action callback"
        );
    }

    Ok(PriceStepUpDeclineResponse {
        declined: true,
        cancellation_effective_at,
    })
}

struct SubscriptionCallbackData {
    subscription_id: String,
    external_user_id: String,
    provider: String,
    purchase_token: Option<String>,
    auto_renewing: Option<bool>,
    current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    status: String,
    revocation_reason: Option<String>,
    google_price_step_up_consent_deadline: Option<chrono::DateTime<chrono::Utc>>,
    google_pause_scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    google_deferred_until: Option<chrono::DateTime<chrono::Utc>>,
}

async fn dispatch_subscription_callback<R: SubscriptionActionsHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    sub: &SubscriptionCallbackData,
    event_type: &str,
    cancellation_mode: Option<&str>,
    new_price_cents: Option<i32>,
) -> Result<(), BridgeError>
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
        "google_price_step_up_consent_deadline": sub.google_price_step_up_consent_deadline.map(|d| d.timestamp_millis()),
        "google_pause_scheduled_at": sub.google_pause_scheduled_at.map(|d| d.timestamp_millis()),
        "google_deferred_until": sub.google_deferred_until.map(|d| d.timestamp_millis()),
    });

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
        new_price_cents: new_price_cents.map(i64::from),
        auto_renewing: sub.auto_renewing,
        purchase_token: sub.purchase_token.clone(),
        current_period_end: sub.current_period_end.map(|d| d.to_rfc3339()),
        status: Some(sub.status.clone()),
        provider: sub.provider.clone(),
        provider_event_id: provider_event_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: sub.revocation_reason.clone(),
        cancellation_mode: cancellation_mode.map(str::to_string),
        google_price_step_up_consent_deadline: sub.google_price_step_up_consent_deadline.map(|d| d.timestamp_millis()),
        google_pause_scheduled_at: sub.google_pause_scheduled_at.map(|d| d.timestamp_millis()),
        google_deferred_until: sub.google_deferred_until.map(|d| d.timestamp_millis()),
        google_pending_price_change_new_price_cents: None,
        google_pending_price_change_currency: None,
        google_pending_price_change_mode: None,
        google_pending_price_change_state: None,
        google_pending_price_change_expected_at: None,
    };

    crate::webhooks::forwarding::create_and_forward_webhook(
        repo,
        app_id,
        &sub.provider,
        &provider_event_id,
        event_type,
        Some(sub.subscription_id.clone()),
        sub.purchase_token.clone(),
        payload,
        Some(timestamp_epoch_ms),
        canonical,
    )
    .await
}

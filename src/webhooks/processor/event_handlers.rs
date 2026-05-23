use std::time::Duration;

use crate::{
    application::{
        app_context::AppSnapshot,
        verify_purchase_provider::acknowledge_google_play,
        verify_purchase_types::ProductType,
    },
    error::BridgeError,
    ports::{
        SubscriptionWebhookTransition, WebhookPaymentRecordRequest,
        WebhookProcessingRepository, WebhookProviderSnapshot,
        WebhookSubscriptionCommitRequest, WebhookSubscriptionSnapshot,
    },
    services::google_play::subscription_lifecycle::GooglePlayLifecycleOutcome,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::{
    normalize_status, parse_rfc3339_utc, send_dispute_admin_alert_email,
    status_to_canonical_event, WebhookFields,
};

const LIFECYCLE_EMAIL_LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(500);

pub(super) struct EventContext<'a> {
    pub(super) app: &'a AppSnapshot,
    pub(super) app_id: Uuid,
    pub(super) canonical_event: &'a str,
    pub(super) provider: &'a str,
    pub(super) webhook: &'a WebhookProviderSnapshot,
    pub(super) fields: &'a WebhookFields,
    pub(super) external_user_id: &'a Option<String>,
    pub(super) timestamp_epoch_ms: i64,
}

pub(super) enum EventHandling {
    Handled(EventEffects),
    ReturnNone,
    NotHandled,
}

pub(super) struct EventEffects {
    pub(super) callback_event_type: Option<String>,
    pub(super) callback_status_override: Option<String>,
    pub(super) callback_revocation_reason_override: Option<String>,
    pub(super) callback_cancellation_mode_override: Option<String>,
    pub(super) canonical_subscription: Option<WebhookSubscriptionSnapshot>,
    pub(super) should_forward: bool,
}

impl Default for EventEffects {
    fn default() -> Self {
        Self {
            callback_event_type: None,
            callback_status_override: None,
            callback_revocation_reason_override: None,
            callback_cancellation_mode_override: None,
            canonical_subscription: None,
            should_forward: true,
        }
    }
}

fn effects_from_google_lifecycle_outcome(outcome: GooglePlayLifecycleOutcome) -> EventEffects {
    EventEffects {
        callback_event_type: outcome.callback_event_type,
        callback_status_override: outcome.callback_status_override,
        callback_revocation_reason_override: outcome.callback_revocation_reason_override,
        callback_cancellation_mode_override: outcome.callback_cancellation_mode_override,
        canonical_subscription: outcome.canonical_subscription,
        should_forward: true,
    }
}

async fn lookup_lifecycle_email(ctx: &EventContext<'_>, event_type: &str) -> Option<String> {
    let user_id = ctx.external_user_id.as_deref()?;

    match lookup_lifecycle_email_once(ctx, user_id).await {
        Ok(email) => email,
        Err(first_error) => {
            warn!(
                "Lifecycle email lookup failed for event {}; retrying once: {}",
                event_type,
                first_error
            );
            tokio::time::sleep(LIFECYCLE_EMAIL_LOOKUP_RETRY_DELAY).await;

            match lookup_lifecycle_email_once(ctx, user_id).await {
                Ok(email) => email,
                Err(second_error) => {
                    error!(
                        "Skipping lifecycle email after lookup retry failed: app_id={}, provider={}, event_type={}, provider_webhook_id={}, external_user_id={}, error={}",
                        ctx.app_id,
                        ctx.provider,
                        event_type,
                        ctx.webhook.provider_webhook_id,
                        user_id,
                        second_error
                    );
                    None
                }
            }
        }
    }
}

async fn lookup_lifecycle_email_once(
    ctx: &EventContext<'_>,
    user_id: &str,
) -> Result<Option<String>, BridgeError> {
    crate::services::email_lookup::lookup_user_email(ctx.app, user_id).await
}

async fn acknowledge_google_play_one_time_from_webhook<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    ctx: &EventContext<'_>,
) -> Result<(), BridgeError> {
    if ctx.provider != "google_play" {
        return Ok(());
    }

    let Some(purchase_token) = ctx.fields.purchase_token.as_deref()
        .or(ctx.webhook.purchase_token.as_deref())
    else {
        warn!(
            "Skipping Google Play OTP acknowledgement for webhook {}: missing purchase token",
            ctx.webhook.provider_webhook_id
        );
        return Ok(());
    };

    let Some(product_id) = ctx.fields.product_id.as_deref()
        .or(ctx.fields.subscription_id.as_deref())
        .or(ctx.webhook.subscription_id.as_deref())
    else {
        warn!(
            "Skipping Google Play OTP acknowledgement for webhook {}: missing product id",
            ctx.webhook.provider_webhook_id
        );
        return Ok(());
    };

    if repo.payment_acknowledged_at(ctx.app_id, ctx.provider, purchase_token).await?.is_some() {
        return Ok(());
    }

    let provider_config = match repo.get_provider_config(ctx.app_id, "google_play").await {
        Ok(config) => config,
        Err(err) => {
            warn!(
                "Skipping Google Play OTP acknowledgement for webhook {}: provider config unavailable: {}",
                ctx.webhook.provider_webhook_id,
                err
            );
            return Ok(());
        }
    };

    if let Err(err) = acknowledge_google_play(
        product_id,
        purchase_token,
        ProductType::OneTimeProduct,
        &provider_config.config,
    )
    .await
    {
        warn!(
            "Google Play OTP acknowledgement failed for webhook {} product {}: {}",
            ctx.webhook.provider_webhook_id,
            product_id,
            err
        );
        return Ok(());
    }

    repo.mark_payment_acknowledged(ctx.app_id, ctx.provider, purchase_token).await
}

async fn acknowledge_google_play_subscription_from_webhook<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    ctx: &EventContext<'_>,
    subscription_id: &str,
) -> Result<(), BridgeError> {
    if ctx.provider != "google_play" || ctx.webhook.event_type != "SUBSCRIPTION_PURCHASED" {
        return Ok(());
    }

    if ctx.fields.status.as_deref() == Some("pending") || ctx.fields.google_subscription_state == Some(5) {
        return Ok(());
    }

    let Some(purchase_token) = ctx.fields.purchase_token.as_deref()
        .or(ctx.webhook.purchase_token.as_deref())
    else {
        warn!(
            "Skipping Google Play subscription acknowledgement for webhook {}: missing purchase token",
            ctx.webhook.provider_webhook_id
        );
        return Ok(());
    };

    if repo.payment_acknowledged_at(ctx.app_id, ctx.provider, purchase_token).await?.is_some() {
        return Ok(());
    }

    let provider_config = match repo.get_provider_config(ctx.app_id, "google_play").await {
        Ok(config) => config,
        Err(err) => {
            warn!(
                "Skipping Google Play subscription acknowledgement for webhook {}: provider config unavailable: {}",
                ctx.webhook.provider_webhook_id,
                err
            );
            return Ok(());
        }
    };

    if let Err(err) = acknowledge_google_play(
        subscription_id,
        purchase_token,
        ProductType::Subscription,
        &provider_config.config,
    )
    .await
    {
        warn!(
            "Google Play subscription acknowledgement failed for webhook {} subscription {}: {}",
            ctx.webhook.provider_webhook_id,
            subscription_id,
            err
        );
        return Ok(());
    }

    repo.mark_payment_acknowledged(ctx.app_id, ctx.provider, purchase_token).await
}

async fn send_price_step_up_email(ctx: &EventContext<'_>, subscription_id: &str) {
    let Some(email) = lookup_lifecycle_email(ctx, "subscription.price_step_up").await else {
        return;
    };
    let Some(new_price_cents) = ctx.fields.google_new_price_cents else {
        warn!("Skipping price step-up email: missing new price");
        return;
    };
    let Some(deadline) = ctx.fields.google_price_step_up_consent_deadline.as_deref().and_then(parse_rfc3339_utc) else {
        warn!("Skipping price step-up email: missing consent deadline");
        return;
    };

    let email_service = crate::services::email::get_email_service();
    if let Err(e) = crate::services::google_play::notifications::send_email_price_step_up(
        email_service.as_ref(),
        &email,
        subscription_id,
        new_price_cents,
        deadline,
    ).await {
        warn!("Failed to send price step-up lifecycle email: {}", e);
    }
}

async fn send_deferred_email(
    ctx: &EventContext<'_>,
    subscription_id: &str,
    deferred_until: chrono::DateTime<chrono::Utc>,
) {
    let Some(email) = lookup_lifecycle_email(ctx, "subscription.deferred").await else {
        return;
    };

    let email_service = crate::services::email::get_email_service();
    if let Err(e) = crate::services::google_play::notifications::send_email_deferred(
        email_service.as_ref(),
        &email,
        subscription_id,
        deferred_until,
    ).await {
        warn!("Failed to send deferred lifecycle email: {}", e);
    }
}

async fn send_paused_email(ctx: &EventContext<'_>, subscription_id: &str) {
    let Some(email) = lookup_lifecycle_email(ctx, "subscription.paused").await else {
        return;
    };

    let email_service = crate::services::email::get_email_service();
    if let Err(e) = crate::services::google_play::notifications::send_email_paused(
        email_service.as_ref(),
        &email,
        subscription_id,
    ).await {
        warn!("Failed to send paused lifecycle email: {}", e);
    }
}

async fn send_resumed_email(ctx: &EventContext<'_>, subscription_id: &str, current_period_end: chrono::DateTime<chrono::Utc>) {
    let Some(email) = lookup_lifecycle_email(ctx, "subscription.resumed").await else {
        return;
    };

    let email_service = crate::services::email::get_email_service();
    if let Err(e) = crate::services::google_play::notifications::send_email_restarted(
        email_service.as_ref(),
        &email,
        subscription_id,
        current_period_end,
    ).await {
        warn!("Failed to send resumed lifecycle email: {}", e);
    }
}

async fn send_refunded_email(ctx: &EventContext<'_>, subscription_id: &str) {
    let Some(email) = lookup_lifecycle_email(ctx, "payment.refunded").await else {
        return;
    };

    let email_service = crate::services::email::get_email_service();
    if let Err(e) = crate::services::google_play::notifications::send_email_refunded(
        email_service.as_ref(),
        &email,
        subscription_id,
    ).await {
        warn!("Failed to send refunded lifecycle email: {}", e);
    }
}

async fn send_payment_failed_email(ctx: &EventContext<'_>, subscription_id: &str) {
    let Some(email) = lookup_lifecycle_email(ctx, "payment.failed").await else {
        return;
    };
    let Some(app_url) = ctx.app.app_url.as_deref() else {
        warn!("Skipping payment failed email: app_url is not configured");
        return;
    };

    let email_service = crate::services::email::get_email_service();
    if let Err(e) = crate::services::google_play::notifications::send_email_payment_failed(
        email_service.as_ref(),
        &email,
        subscription_id,
        ctx.provider,
        app_url,
    ).await {
        warn!("Failed to send payment failed lifecycle email: {}", e);
    }
}

pub(super) async fn handle_subscription_event<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    ctx: &EventContext<'_>,
) -> Result<EventHandling, BridgeError> {
    match ctx.canonical_event {
        "subscription.activated" | "subscription.renewed" | "subscription.recovered" | "subscription.created" => {
            let Some(user_id) = ctx.external_user_id.as_deref() else {
                return Ok(EventHandling::Handled(EventEffects::default()));
            };

            let period_end = ctx.fields.current_period_end.as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let sub_id_fallback = ctx.webhook.subscription_id.clone().unwrap_or_default();
            let sub_id_str = ctx.fields.subscription_id.as_deref().unwrap_or(&sub_id_fallback);

            let (adopt_stale_payment, stale_payment_window_secs) = if ctx.provider == "creem" {
                let window_secs = repo
                    .get_provider_config(ctx.app_id, "creem")
                    .await
                    .ok()
                    .and_then(|cfg| cfg.config.get("stale_payment_window_secs").and_then(|v| v.as_i64()))
                    .unwrap_or(86400);
                (true, window_secs)
            } else {
                (false, 86400)
            };

            let subscription = repo
                .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    subscription_id: sub_id_str,
                    provider: ctx.provider,
                    status: "active",
                    current_period_end: period_end,
                    purchase_token: ctx.fields.purchase_token.as_deref(),
                    auto_renewing: ctx.fields.auto_renewing,
                    payment_state: None,
                    provider_customer_id: ctx.fields.provider_customer_id.as_deref(),
                    event_time_ms: ctx.timestamp_epoch_ms,
                    recurring_amount_cents: ctx.fields.amount_cents.map(i64::from),
                    payment: Some(WebhookPaymentRecordRequest {
                        app_id: ctx.app_id,
                        external_user_id: user_id,
                        provider: ctx.provider,
                        provider_transaction_id: ctx.fields.provider_transaction_id.as_deref()
                            .unwrap_or(&ctx.webhook.provider_webhook_id),
                        subscription_id: ctx.fields.subscription_id.as_deref(),
                        product_id: ctx.fields.product_id.as_deref(),
                        amount_cents: ctx.fields.amount_cents.unwrap_or(0),
                        currency: ctx.fields.currency.as_deref(),
                        status: "success",
                    }),
                    adopt_stale_payment,
                    stale_payment_window_secs,
                })
                .await?;

            let Some(subscription) = subscription else {
                info!(
                    "Skipped stale activation event for subscription {} (provider: {})",
                    sub_id_str,
                    ctx.webhook.provider
                );
                return Ok(EventHandling::ReturnNone);
            };

            acknowledge_google_play_subscription_from_webhook(repo, ctx, sub_id_str).await?;

            if ctx.provider == "google_play" {
                let _ = repo
                    .link_replacement_subscriptions(ctx.app_id, user_id, sub_id_str, ctx.timestamp_epoch_ms)
                    .await;
            }

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("subscription.activated".to_string()),
                callback_status_override: Some("active".to_string()),
                canonical_subscription: Some(subscription),
                ..Default::default()
            }))
        }

        "subscription.pending" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let sub_id = ctx.fields.subscription_id.as_deref()
                    .or(ctx.webhook.subscription_id.as_deref())
                    .unwrap_or("");
                let updated = repo.apply_subscription_transition(
                    ctx.app_id,
                    user_id,
                    ctx.provider,
                    sub_id,
                    ctx.timestamp_epoch_ms,
                    SubscriptionWebhookTransition::Pending,
                ).await?;
                if updated.is_none() {
                    info!("Skipped stale pending event for subscription {}", sub_id);
                    return Ok(EventHandling::ReturnNone);
                }
            }

            Ok(EventHandling::Handled(EventEffects {
                should_forward: false,
                ..Default::default()
            }))
        }

        "subscription.trial_started" => {
            let Some(user_id) = ctx.external_user_id.as_deref() else {
                return Ok(EventHandling::Handled(EventEffects::default()));
            };

            let period_end = ctx.fields.current_period_end.as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let sub_id_fallback = ctx.webhook.subscription_id.clone().unwrap_or_default();
            let sub_id_str = ctx.fields.subscription_id.as_deref().unwrap_or(&sub_id_fallback);

            let subscription = repo
                .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    subscription_id: sub_id_str,
                    provider: ctx.provider,
                    status: "trial",
                    current_period_end: period_end,
                    purchase_token: ctx.fields.purchase_token.as_deref(),
                    auto_renewing: ctx.fields.auto_renewing,
                    payment_state: None,
                    provider_customer_id: ctx.fields.provider_customer_id.as_deref(),
                    event_time_ms: ctx.timestamp_epoch_ms,
                    recurring_amount_cents: ctx.fields.amount_cents.map(i64::from),
                    payment: None,
                    adopt_stale_payment: false,
                    stale_payment_window_secs: 86400,
                })
                .await?;

            let Some(subscription) = subscription else {
                info!(
                    "Skipped stale trial-start event for subscription {} (provider: {})",
                    sub_id_str,
                    ctx.provider
                );
                return Ok(EventHandling::ReturnNone);
            };

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("subscription.activated".to_string()),
                callback_status_override: Some("trial".to_string()),
                canonical_subscription: Some(subscription),
                ..Default::default()
            }))
        }

        "subscription.grace_period" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let sub_id = ctx.fields.subscription_id.clone().unwrap_or_default();
                let grace_end = ctx.fields.current_period_end.as_deref().and_then(parse_rfc3339_utc);
                let updated = repo.apply_subscription_transition(
                    ctx.app_id,
                    user_id,
                    ctx.provider,
                    &sub_id,
                    ctx.timestamp_epoch_ms,
                    SubscriptionWebhookTransition::GracePeriod {
                        grace_period_end: grace_end,
                    },
                ).await?;

                if let Some(sub) = updated {
                    return Ok(EventHandling::Handled(EventEffects {
                        callback_event_type: Some("subscription.grace_period".to_string()),
                        callback_status_override: Some("past_due".to_string()),
                        canonical_subscription: Some(sub.into()),
                        ..Default::default()
                    }));
                }

                info!("Skipped stale grace_period event for subscription {}", sub_id);
                return Ok(EventHandling::ReturnNone);
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.revoked" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_revoked(
                    repo,
                    ctx.app_id,
                    user_id,
                    ctx.webhook,
                    ctx.fields,
                    ctx.timestamp_epoch_ms,
                ).await? else {
                    return Ok(EventHandling::ReturnNone);
                };

                return Ok(EventHandling::Handled(effects_from_google_lifecycle_outcome(outcome)));
            }
            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.on_hold" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let sub_id = ctx.fields.subscription_id.as_deref()
                    .or(ctx.webhook.subscription_id.as_deref())
                    .unwrap_or("");
                let updated = repo.apply_subscription_transition(
                    ctx.app_id,
                    user_id,
                    ctx.provider,
                    sub_id,
                    ctx.timestamp_epoch_ms,
                    SubscriptionWebhookTransition::OnHold,
                ).await?;

                if let Some(sub) = updated {
                    return Ok(EventHandling::Handled(EventEffects {
                        callback_event_type: Some("subscription.on_hold".to_string()),
                        callback_status_override: Some("on_hold".to_string()),
                        canonical_subscription: Some(sub.into()),
                        ..Default::default()
                    }));
                }

                info!("Skipped stale on_hold event for subscription {}", sub_id);
                return Ok(EventHandling::ReturnNone);
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.paused" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let sub_id = ctx.fields.subscription_id.clone().unwrap_or_default();
                if let Ok(Some(sub)) = repo.get_subscription_by_sub_id_and_user_for_provider(ctx.app_id, ctx.provider, &sub_id, user_id).await {
                    if sub.status == "active" || sub.status == "trial" {
                        let updated = repo.apply_subscription_transition(
                            ctx.app_id,
                            user_id,
                            ctx.provider,
                            &sub_id,
                            ctx.timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Paused,
                        ).await?;

                        if let Some(updated_sub) = updated {
                            send_paused_email(ctx, &sub_id).await;
                            return Ok(EventHandling::Handled(EventEffects {
                                callback_event_type: Some("subscription.paused".to_string()),
                                callback_status_override: Some("paused".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        }

                        info!("Skipped stale paused event for subscription {}", sub_id);
                        return Ok(EventHandling::ReturnNone);
                    }

                    info!(
                        "Ignoring pause event for subscription {} in status '{}' (not active/trial)",
                        sub_id,
                        sub.status
                    );
                }
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.resumed" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let outcome = crate::services::google_play::subscription_lifecycle::handle_subscription_resumed(
                    repo,
                    ctx.app_id,
                    user_id,
                    ctx.webhook,
                    ctx.fields,
                    ctx.timestamp_epoch_ms,
                ).await?;

                if outcome.is_some() {
                    if let Some(sub_id) = ctx.fields.subscription_id.as_deref().or(ctx.webhook.subscription_id.as_deref()) {
                        let period_end = outcome.as_ref()
                            .and_then(|o| o.canonical_subscription.as_ref())
                            .and_then(|s| s.current_period_end)
                            .unwrap_or_else(chrono::Utc::now);
                        send_resumed_email(ctx, sub_id, period_end).await;
                    }
                }

                return Ok(EventHandling::Handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()));
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.cancellation_scheduled" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let outcome = crate::services::google_play::subscription_lifecycle::handle_subscription_cancellation_scheduled(
                    repo,
                    ctx.app_id,
                    user_id,
                    ctx.webhook,
                    ctx.fields,
                    ctx.timestamp_epoch_ms,
                ).await?;

                return Ok(EventHandling::Handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()));
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.expired" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                if let Some(purchase_token) = ctx.fields.purchase_token.as_deref() {
                    if let Some(sub) = repo.get_subscription_by_purchase_token_for_provider(ctx.app_id, ctx.provider, purchase_token).await? {
                        let updated = repo.apply_subscription_transition(
                            ctx.app_id,
                            user_id,
                            ctx.provider,
                            &sub.subscription_id,
                            ctx.timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Expired,
                        ).await?;

                        if let Some(updated_sub) = updated {
                            return Ok(EventHandling::Handled(EventEffects {
                                callback_event_type: Some("subscription.expired".to_string()),
                                callback_status_override: Some("expired".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        }
                    } else {
                        let sub_id = ctx.fields.subscription_id.clone().unwrap_or_default();
                        let updated = repo.apply_subscription_transition(
                            ctx.app_id,
                            user_id,
                            ctx.provider,
                            &sub_id,
                            ctx.timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Expired,
                        ).await?;

                        if let Some(updated_sub) = updated {
                            return Ok(EventHandling::Handled(EventEffects {
                                callback_event_type: Some("subscription.expired".to_string()),
                                callback_status_override: Some("expired".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        }

                        info!("Skipped stale expired event for subscription {}", sub_id);
                        return Ok(EventHandling::ReturnNone);
                    }
                } else if let Some(user_id) = ctx.external_user_id.as_deref() {
                    let sub_id = ctx.fields.subscription_id.clone().unwrap_or_default();
                    let updated = repo.apply_subscription_transition(
                        ctx.app_id,
                        user_id,
                        ctx.provider,
                        &sub_id,
                        ctx.timestamp_epoch_ms,
                        SubscriptionWebhookTransition::Expired,
                    ).await?;

                    if let Some(updated_sub) = updated {
                        return Ok(EventHandling::Handled(EventEffects {
                            callback_event_type: Some("subscription.expired".to_string()),
                            callback_status_override: Some("expired".to_string()),
                            canonical_subscription: Some(updated_sub.into()),
                            ..Default::default()
                        }));
                    }

                    info!("Skipped stale expired event for subscription {}", sub_id);
                    return Ok(EventHandling::ReturnNone);
                }
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.cancelled" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let outcome = crate::services::google_play::subscription_lifecycle::handle_subscription_cancelled_with_context(
                    repo,
                    ctx.app_id,
                    user_id,
                    ctx.webhook,
                    ctx.fields,
                    ctx.timestamp_epoch_ms,
                ).await?;

                return Ok(EventHandling::Handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()));
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.pending_purchase_cancelled" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_pending_purchase_cancelled(
                    repo,
                    ctx.app_id,
                    user_id,
                    ctx.webhook,
                    ctx.fields,
                    ctx.timestamp_epoch_ms,
                ).await? else {
                    return Ok(EventHandling::ReturnNone);
                };

                return Ok(EventHandling::Handled(effects_from_google_lifecycle_outcome(outcome)));
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.updated" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let status = normalize_status(ctx.fields.status.as_deref());
                let sub_id = ctx.fields.subscription_id.clone()
                    .or(ctx.webhook.subscription_id.clone())
                    .unwrap_or_default();

                let subscription = repo
                    .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                        app_id: ctx.app_id,
                        external_user_id: user_id,
                        subscription_id: &sub_id,
                        provider: ctx.provider,
                        status: &status,
                        current_period_end: ctx.fields.current_period_end.as_deref().and_then(parse_rfc3339_utc),
                        purchase_token: ctx.fields.purchase_token.as_deref(),
                        auto_renewing: ctx.fields.auto_renewing,
                        payment_state: None,
                        provider_customer_id: ctx.fields.provider_customer_id.as_deref(),
                        event_time_ms: ctx.timestamp_epoch_ms,
                        recurring_amount_cents: ctx.fields.amount_cents.map(i64::from),
                        payment: None,
                        adopt_stale_payment: false,
                        stale_payment_window_secs: 86400,
                    })
                    .await?;

                let Some(subscription) = subscription else {
                    return Ok(EventHandling::ReturnNone);
                };

                return Ok(EventHandling::Handled(EventEffects {
                    callback_event_type: status_to_canonical_event(&status),
                    callback_status_override: Some(status),
                    canonical_subscription: Some(subscription),
                    ..Default::default()
                }));
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.price_step_up" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let outcome = crate::services::google_play::subscription_lifecycle::handle_price_step_up_consent_required(
                    repo,
                    ctx.app_id,
                    user_id,
                    ctx.webhook,
                    ctx.fields,
                    ctx.timestamp_epoch_ms,
                ).await?;

                if outcome.is_some() {
                    if let Some(sub_id) = ctx.fields.subscription_id.as_deref().or(ctx.webhook.subscription_id.as_deref()) {
                        send_price_step_up_email(ctx, sub_id).await;
                    }
                }

                return Ok(EventHandling::Handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()));
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.pause_scheduled" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                if let Some(sub_id) = ctx.fields.subscription_id.as_deref().or(ctx.webhook.subscription_id.as_deref()) {
                    let pause_scheduled_at = ctx.webhook.payload.pointer("/subscriptionNotification/pauseScheduleTimeMillis")
                        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);

                    if let Some(schedule_at) = pause_scheduled_at {
                        let updated = repo.apply_subscription_transition(
                            ctx.app_id,
                            user_id,
                            ctx.provider,
                            sub_id,
                            ctx.timestamp_epoch_ms,
                            SubscriptionWebhookTransition::PauseScheduled {
                                google_pause_scheduled_at: schedule_at,
                            },
                        ).await?;

                        if let Some(updated_sub) = updated {
                            return Ok(EventHandling::Handled(EventEffects {
                                callback_status_override: Some("active".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        }

                        info!("Skipped stale pause_scheduled event for subscription {}", sub_id);
                    }
                }
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.deferred" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                if let Some(sub_id) = ctx.fields.subscription_id.as_deref().or(ctx.webhook.subscription_id.as_deref()) {
                    let deferred_until = ctx.webhook.payload.pointer("/subscriptionNotification/deferredExpiryTimeMillis")
                        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);
                    if let Some(until) = deferred_until {
                        let updated = repo.apply_subscription_transition(
                            ctx.app_id,
                            user_id,
                            ctx.provider,
                            sub_id,
                            ctx.timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Deferred {
                                google_deferred_until: until,
                            },
                        ).await?;

                        if let Some(updated_sub) = updated {
                            send_deferred_email(ctx, sub_id, until).await;
                            return Ok(EventHandling::Handled(EventEffects {
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        } else {
                            info!("Skipped stale deferred event for subscription {}", sub_id);
                        }
                    }
                }
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.price_changed" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let txn_id = ctx.fields.provider_transaction_id.as_deref()
                    .or(ctx.fields.subscription_id.as_deref())
                    .unwrap_or(&ctx.webhook.provider_webhook_id);
                let sub_id = ctx.fields.subscription_id.as_deref();
                let existing_currency = if ctx.fields.currency.is_none() {
                    if let Some(sub_id) = sub_id {
                        repo.get_payment_currency_for_subscription(
                            ctx.app_id,
                            ctx.provider,
                            user_id,
                            sub_id,
                        )
                        .await?
                    } else {
                        None
                    }
                } else {
                    None
                };
                let currency = ctx.fields.currency.as_deref().or(existing_currency.as_deref());
                let _ = repo
                    .record_webhook_payment(WebhookPaymentRecordRequest {
                        app_id: ctx.app_id,
                        external_user_id: user_id,
                        provider: ctx.provider,
                        provider_transaction_id: txn_id,
                        subscription_id: sub_id,
                        product_id: ctx.fields.product_id.as_deref(),
                        amount_cents: ctx.fields.amount_cents.unwrap_or(0),
                        currency,
                        status: "price_changed",
                    })
                    .await;
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.price_change_updated" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let sub_id = ctx.fields.subscription_id.as_deref()
                    .or(ctx.webhook.subscription_id.as_deref())
                    .unwrap_or("");

                let expected_at = ctx.fields.google_pending_price_change_expected_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc);

                let updated = repo.apply_subscription_transition(
                    ctx.app_id,
                    user_id,
                    ctx.provider,
                    sub_id,
                    ctx.timestamp_epoch_ms,
                    SubscriptionWebhookTransition::PendingPriceChange {
                        new_price_cents: ctx.fields.google_pending_price_change_new_price_cents,
                        currency: ctx.fields.google_pending_price_change_currency.clone(),
                        mode: ctx.fields.google_pending_price_change_mode.clone(),
                        state: ctx.fields.google_pending_price_change_state.clone(),
                        expected_at,
                    },
                ).await?;

                if let Some(updated_sub) = updated {
                    return Ok(EventHandling::Handled(EventEffects {
                        callback_event_type: Some("subscription.price_change_updated".to_string()),
                        callback_status_override: Some(updated_sub.status.clone()),
                        canonical_subscription: Some(updated_sub.into()),
                        ..Default::default()
                    }));
                }

                info!("Skipped stale price_change_updated event for subscription {}", sub_id);
                return Ok(EventHandling::ReturnNone);
            }

            Ok(EventHandling::Handled(EventEffects::default()))
        }

        "subscription.expired_voided" => {
            info!(
                "Processed informational webhook event: {} (provider: {})",
                ctx.canonical_event,
                ctx.webhook.provider
            );
            Ok(EventHandling::Handled(EventEffects::default()))
        }

        _ => Ok(EventHandling::NotHandled),
    }
}

pub(super) async fn handle_payment_event<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    ctx: &EventContext<'_>,
) -> Result<EventHandling, BridgeError> {
    match ctx.canonical_event {
        "payment.pending" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let txn_id = ctx.fields.provider_transaction_id.as_deref()
                    .or(ctx.fields.subscription_id.as_deref())
                    .unwrap_or(&ctx.webhook.provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    provider: ctx.provider,
                    provider_transaction_id: txn_id,
                    subscription_id: ctx.fields.subscription_id.as_deref(),
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(0),
                    currency: ctx.fields.currency.as_deref(),
                    status: "pending",
                })
                .await?;
            }

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("payment.pending".to_string()),
                callback_status_override: Some("pending".to_string()),
                ..Default::default()
            }))
        }

        "payment.failed" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let sub_id = ctx.fields.subscription_id.as_deref()
                    .or(ctx.webhook.subscription_id.as_deref())
                    .unwrap_or("");

                let txn_id = ctx.fields.provider_transaction_id.as_deref()
                    .or(ctx.fields.subscription_id.as_deref())
                    .unwrap_or(&ctx.webhook.provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    provider: ctx.provider,
                    provider_transaction_id: txn_id,
                    subscription_id: if sub_id.is_empty() { None } else { Some(sub_id) },
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(0),
                    currency: ctx.fields.currency.as_deref(),
                    status: "failed",
                })
                .await?;

                if sub_id.is_empty() {
                    return Ok(EventHandling::Handled(EventEffects {
                        callback_event_type: Some("payment.failed".to_string()),
                        callback_status_override: Some("failed".to_string()),
                        ..Default::default()
                    }));
                }

                if repo.get_subscription_by_sub_id_and_user_for_provider(ctx.app_id, ctx.provider, sub_id, user_id).await?.is_none() {
                    warn!(
                        "Skipping order.failed subscription update {}: subscription {} not found for user {}",
                        ctx.webhook.provider_webhook_id,
                        sub_id,
                        user_id
                    );
                    return Ok(EventHandling::Handled(EventEffects {
                        callback_event_type: Some("payment.failed".to_string()),
                        callback_status_override: Some("failed".to_string()),
                        ..Default::default()
                    }));
                }

                let updated = repo
                    .apply_subscription_transition(
                        ctx.app_id,
                        user_id,
                        ctx.provider,
                        sub_id,
                        ctx.timestamp_epoch_ms,
                        SubscriptionWebhookTransition::PaymentFailed,
                    )
                    .await?;

                let Some(updated_sub) = updated else {
                    warn!(
                        "Skipping stale order.failed update for subscription {}",
                        sub_id
                    );
                    return Ok(EventHandling::Handled(EventEffects {
                        callback_event_type: Some("payment.failed".to_string()),
                        callback_status_override: Some("failed".to_string()),
                        ..Default::default()
                    }));
                };

                send_payment_failed_email(ctx, sub_id).await;

                return Ok(EventHandling::Handled(EventEffects {
                    callback_event_type: Some("payment.failed".to_string()),
                    callback_status_override: Some("failed".to_string()),
                    canonical_subscription: Some(updated_sub.into()),
                    ..Default::default()
                }));
            }

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("payment.failed".to_string()),
                callback_status_override: Some("failed".to_string()),
                ..Default::default()
            }))
        }

        "purchase.one_time" => {
            let outcome = crate::services::google_play::product_lifecycle::handle_otp_purchased(
                repo,
                ctx.app_id,
                ctx.webhook,
                ctx.fields,
                ctx.external_user_id.as_deref(),
                ctx.timestamp_epoch_ms,
            ).await?;

            if outcome.is_some() {
                acknowledge_google_play_one_time_from_webhook(repo, ctx).await?;
            }

            Ok(EventHandling::Handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()))
        }

        "purchase.one_time_cancelled" => {
            let Some(outcome) = crate::services::google_play::product_lifecycle::handle_otp_cancelled(
                repo,
                ctx.app_id,
                ctx.webhook,
                ctx.fields,
                ctx.external_user_id.as_deref(),
                ctx.timestamp_epoch_ms,
            ).await? else {
                return Ok(EventHandling::ReturnNone);
            };

            Ok(EventHandling::Handled(effects_from_google_lifecycle_outcome(outcome)))
        }

        "purchase.one_time_refunded" => {
            let Some(outcome) = crate::services::google_play::product_lifecycle::handle_otp_refunded(
                repo,
                ctx.app_id,
                ctx.webhook,
                ctx.fields,
                ctx.external_user_id.as_deref(),
                ctx.timestamp_epoch_ms,
            ).await? else {
                return Ok(EventHandling::ReturnNone);
            };

            Ok(EventHandling::Handled(effects_from_google_lifecycle_outcome(outcome)))
        }

        "payment.refunded" => {
            if let Some(_user_id) = ctx.external_user_id.as_deref() {
                if let Some(token) = ctx.fields.purchase_token.as_deref().or(ctx.webhook.purchase_token.as_deref()) {
                    let existing = repo.get_payment_status_for_provider(ctx.app_id, ctx.provider, token).await?;
                    if existing.as_deref() != Some("refunded") {
                        repo.update_payment_status_for_provider(ctx.app_id, ctx.provider, token, "refunded").await?;
                    }
                    if let Some(sub) = repo.get_subscription_by_purchase_token_for_provider(ctx.app_id, ctx.provider, token).await? {
                        let updated = repo.apply_subscription_transition(
                            ctx.app_id,
                            _user_id,
                            ctx.provider,
                            &sub.subscription_id,
                            ctx.timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Revoked {
                                revocation_reason: Some("REFUND".to_string()),
                            },
                        ).await?;
                        if let Some(updated_sub) = updated {
                            send_refunded_email(ctx, &sub.subscription_id).await;
                            return Ok(EventHandling::Handled(EventEffects {
                                callback_event_type: Some("payment.refunded".to_string()),
                                callback_status_override: Some("refunded".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        }
                    }
                } else if let Some(sub_id) = ctx.fields.subscription_id.as_deref() {
                    let updated = repo.apply_subscription_transition(
                        ctx.app_id,
                        _user_id,
                        ctx.provider,
                        sub_id,
                        ctx.timestamp_epoch_ms,
                        SubscriptionWebhookTransition::Revoked {
                            revocation_reason: Some("REFUND".to_string()),
                        },
                    ).await?;
                    if let Some(_updated_sub) = updated {
                        send_refunded_email(ctx, sub_id).await;
                        return Ok(EventHandling::Handled(EventEffects {
                            callback_event_type: Some("payment.refunded".to_string()),
                            callback_status_override: Some("refunded".to_string()),
                            canonical_subscription: Some(_updated_sub.into()),
                            ..Default::default()
                        }));
                    }
                }
            }

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("payment.refunded".to_string()),
                callback_status_override: Some("refunded".to_string()),
                ..Default::default()
            }))
        }

        "payment.partially_refunded" => {
            if let Some(_user_id) = ctx.external_user_id.as_deref() {
                // For Creem OTP: checkout_id in the payload maps to purchase_token
                if let Some(token) = ctx.fields.purchase_token.as_deref().or(ctx.webhook.purchase_token.as_deref()) {
                    let existing = repo.get_payment_status_for_provider(ctx.app_id, ctx.provider, token).await?;
                    if !matches!(existing.as_deref(), Some("partially_refunded") | Some("refunded")) {
                        repo.update_payment_status_for_provider(ctx.app_id, ctx.provider, token, "partially_refunded").await?;
                    }
                }
            }

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("payment.partially_refunded".to_string()),
                callback_status_override: Some("partially_refunded".to_string()),
                ..Default::default()
            }))
        }

        "dispute.created" => {
            if let Err(e) = send_dispute_admin_alert_email(ctx.app, ctx.webhook, ctx.fields, ctx.external_user_id.as_deref()).await {
                warn!(
                    "Failed to send dispute admin alert for event {}: {}",
                    ctx.webhook.provider_webhook_id,
                    e
                );
            }

            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let txn_id = ctx.fields.provider_transaction_id.as_deref()
                    .or(ctx.webhook.subscription_id.as_deref())
                    .unwrap_or(&ctx.webhook.provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    provider: ctx.provider,
                    provider_transaction_id: txn_id,
                    subscription_id: ctx.fields.subscription_id.as_deref(),
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(0),
                    currency: ctx.fields.currency.as_deref(),
                    status: "dispute_created",
                })
                .await?;
            }

            Ok(EventHandling::Handled(EventEffects {
                callback_event_type: Some("dispute.created".to_string()),
                ..Default::default()
            }))
        }

        _ => Ok(EventHandling::NotHandled),
    }
}

pub(super) async fn handle_provider_event<R: WebhookProcessingRepository + ?Sized>(
    _repo: &R,
    _ctx: &EventContext<'_>,
) -> Result<EventHandling, BridgeError> {
    Ok(EventHandling::NotHandled)
}

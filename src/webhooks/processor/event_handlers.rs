use std::time::Duration;

use crate::{
    application::app_context::AppSnapshot,
    error::BridgeError,
    ports::{
        SubscriptionWebhookTransition, WebhookPaymentRecordRequest,
        WebhookProcessingRepository, WebhookProviderSnapshot,
        WebhookSubscriptionCommitRequest, WebhookSubscriptionSnapshot,
    },
    services::email::EmailContext,
    services::google_play::subscription_lifecycle::GooglePlayLifecycleOutcome,
    utils::diagnostic_hash,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::{
    parse_rfc3339_utc, send_dispute_admin_alert_email,
    status_to_canonical_event, WebhookFields,
};
use crate::webhooks::provider_adapter::{NormalizedProviderStatus, ProviderWebhookAdapter};

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
    Handled(Box<EventEffects>),
    ReturnNone,
    NotHandled,
}

impl EventHandling {
    pub(super) fn handled(effects: EventEffects) -> Self {
        Self::Handled(Box::new(effects))
    }
}

pub(super) struct EventEffects {
    pub(super) callback_event_type: Option<String>,
    pub(super) callback_status_override: Option<String>,
    pub(super) callback_revocation_reason_override: Option<String>,
    pub(super) callback_cancellation_mode_override: Option<String>,
    pub(super) canonical_subscription: Option<WebhookSubscriptionSnapshot>,
    pub(super) should_forward: bool,
    pub(super) post_commit: Vec<PostCommitEffect>,
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
            post_commit: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum PostCommitEffect {
    LifecycleEmail(Box<LifecycleEmailEffect>),
    DisputeAdminAlert(Box<DisputeAdminAlertEffect>),
}

#[derive(Debug, Clone)]
pub(super) struct LifecycleEmailEffect {
    app: AppSnapshot,
    app_id: Uuid,
    provider: String,
    provider_webhook_id: String,
    external_user_id: String,
    subscription_id: String,
    kind: LifecycleEmailKind,
}

#[derive(Debug, Clone)]
pub(super) enum LifecycleEmailKind {
    PriceStepUp {
        new_price_cents: i64,
        deadline: chrono::DateTime<chrono::Utc>,
    },
    Deferred {
        deferred_until: chrono::DateTime<chrono::Utc>,
    },
    Paused,
    Resumed {
        current_period_end: chrono::DateTime<chrono::Utc>,
    },
    Refunded,
    PaymentFailed {
        app_url: String,
    },
}

#[derive(Debug, Clone)]
pub(super) struct DisputeAdminAlertEffect {
    app: AppSnapshot,
    webhook: WebhookProviderSnapshot,
    fields: WebhookFields,
    external_user_id: Option<String>,
}

fn effects_from_google_lifecycle_outcome(outcome: GooglePlayLifecycleOutcome) -> EventEffects {
    EventEffects {
        callback_event_type: outcome.callback_event_type,
        callback_status_override: outcome.callback_status_override,
        callback_revocation_reason_override: outcome.callback_revocation_reason_override,
        callback_cancellation_mode_override: outcome.callback_cancellation_mode_override,
        canonical_subscription: outcome.canonical_subscription,
        should_forward: true,
        post_commit: Vec::new(),
    }
}

pub(super) fn activation_subscription_status(provider: &str, raw_status: Option<&str>) -> Option<String> {
    if provider == "creem" {
        match ProviderWebhookAdapter::Creem.normalize_status(raw_status) {
            NormalizedProviderStatus::Known(status) => Some(status),
            NormalizedProviderStatus::Missing => Some("active".to_string()),
            NormalizedProviderStatus::Unknown(_) => None,
        }
    } else {
        Some("active".to_string())
    }
}

impl LifecycleEmailKind {
    fn event_type(&self) -> &'static str {
        match self {
            Self::PriceStepUp { .. } => "subscription.price_step_up",
            Self::Deferred { .. } => "subscription.deferred",
            Self::Paused => "subscription.paused",
            Self::Resumed { .. } => "subscription.resumed",
            Self::Refunded => "payment.refunded",
            Self::PaymentFailed { .. } => "payment.failed",
        }
    }
}

impl LifecycleEmailEffect {
    fn idempotency_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.app_id,
            self.provider,
            self.provider_webhook_id,
            self.kind.event_type(),
            self.subscription_id
        )
    }
}

fn lifecycle_email_context<'a>(
    effect: &'a LifecycleEmailEffect,
    event_type: &'a str,
    idempotency_key: &'a str,
) -> EmailContext<'a> {
    EmailContext {
        email_type: Some("lifecycle"),
        app_id: Some(effect.app_id),
        provider: Some(effect.provider.as_str()),
        event_type: Some(event_type),
        provider_webhook_id: Some(effect.provider_webhook_id.as_str()),
        external_user_id: Some(effect.external_user_id.as_str()),
        subscription_id: Some(effect.subscription_id.as_str()),
        idempotency_key: Some(idempotency_key),
    }
}

fn log_lifecycle_email_failure(
    effect: &LifecycleEmailEffect,
    event_type: &str,
    idempotency_key: &str,
    error: &BridgeError,
) {
    let external_user_id_hash = diagnostic_hash(&effect.external_user_id);
    warn!(
        app_id = %effect.app_id,
        provider = effect.provider,
        event_type,
        provider_webhook_id = %effect.provider_webhook_id,
        external_user_id_hash = %external_user_id_hash,
        subscription_id = %effect.subscription_id,
        email_idempotency_key = %idempotency_key,
        error = %error,
        "Failed to send lifecycle email"
    );
}

async fn lookup_lifecycle_email(
    effect: &LifecycleEmailEffect,
    event_type: &str,
) -> Option<String> {
    let external_user_id_hash = diagnostic_hash(&effect.external_user_id);

    match lookup_lifecycle_email_once(effect, event_type).await {
        Ok(email) => email,
        Err(first_error) => {
            warn!(
                app_id = %effect.app_id,
                provider = effect.provider,
                event_type,
                provider_webhook_id = %effect.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = %effect.subscription_id,
                error = %first_error,
                "Lifecycle email lookup failed; retrying once"
            );
            tokio::time::sleep(LIFECYCLE_EMAIL_LOOKUP_RETRY_DELAY).await;

            match lookup_lifecycle_email_once(effect, event_type).await {
                Ok(email) => email,
                Err(second_error) => {
                    error!(
                        app_id = %effect.app_id,
                        provider = effect.provider,
                        event_type,
                        provider_webhook_id = %effect.provider_webhook_id,
                        external_user_id_hash = %external_user_id_hash,
                        subscription_id = %effect.subscription_id,
                        error = %second_error,
                        "Skipping lifecycle email after lookup retry failed"
                    );
                    None
                }
            }
        }
    }
}

async fn lookup_lifecycle_email_once(
    effect: &LifecycleEmailEffect,
    event_type: &str,
) -> Result<Option<String>, BridgeError> {
    crate::services::email_lookup::lookup_user_email_with_context(
        &effect.app,
        &effect.external_user_id,
        crate::services::email_lookup::EmailLookupContext {
            origin: "lifecycle_email",
            event_type: Some(event_type),
            provider: Some(effect.provider.as_str()),
            provider_webhook_id: Some(effect.provider_webhook_id.as_str()),
            subscription_id: Some(effect.subscription_id.as_str()),
        },
    ).await
}

pub(super) async fn execute_post_commit_effect(effect: PostCommitEffect) {
    match effect {
        PostCommitEffect::LifecycleEmail(effect) => execute_lifecycle_email_effect(*effect).await,
        PostCommitEffect::DisputeAdminAlert(effect) => {
            if let Err(e) = send_dispute_admin_alert_email(
                &effect.app,
                &effect.webhook,
                &effect.fields,
                effect.external_user_id.as_deref(),
            ).await {
                warn!(
                    "Failed to send dispute admin alert for event {}: {}",
                    effect.webhook.provider_webhook_id,
                    e
                );
            }
        }
    }
}

async fn execute_lifecycle_email_effect(effect: LifecycleEmailEffect) {
    let event_type = effect.kind.event_type();
    let Some(email) = lookup_lifecycle_email(&effect, event_type).await else {
        return;
    };
    let idempotency_key = effect.idempotency_key();
    let context = lifecycle_email_context(&effect, event_type, &idempotency_key);
    let email_service = crate::services::email::get_email_service();

    let result = match &effect.kind {
        LifecycleEmailKind::PriceStepUp { new_price_cents, deadline } => {
            crate::services::google_play::notifications::send_email_price_step_up(
                email_service.as_ref(),
                &email,
                &effect.subscription_id,
                *new_price_cents,
                *deadline,
                context,
            ).await
        }
        LifecycleEmailKind::Deferred { deferred_until } => {
            crate::services::google_play::notifications::send_email_deferred(
                email_service.as_ref(),
                &email,
                &effect.subscription_id,
                *deferred_until,
                context,
            ).await
        }
        LifecycleEmailKind::Paused => {
            crate::services::google_play::notifications::send_email_paused(
                email_service.as_ref(),
                &email,
                &effect.subscription_id,
                context,
            ).await
        }
        LifecycleEmailKind::Resumed { current_period_end } => {
            crate::services::google_play::notifications::send_email_restarted(
                email_service.as_ref(),
                &email,
                &effect.subscription_id,
                *current_period_end,
                context,
            ).await
        }
        LifecycleEmailKind::Refunded => {
            crate::services::google_play::notifications::send_email_refunded(
                email_service.as_ref(),
                &email,
                &effect.subscription_id,
                context,
            ).await
        }
        LifecycleEmailKind::PaymentFailed { app_url } => {
            crate::services::google_play::notifications::send_email_payment_failed(
                email_service.as_ref(),
                &email,
                &effect.subscription_id,
                &effect.provider,
                app_url,
                context,
            ).await
        }
    };

    if let Err(e) = result {
        log_lifecycle_email_failure(&effect, event_type, &idempotency_key, &e);
    }
}

fn lifecycle_email_effect(
    ctx: &EventContext<'_>,
    subscription_id: &str,
    kind: LifecycleEmailKind,
) -> Option<PostCommitEffect> {
    Some(PostCommitEffect::LifecycleEmail(Box::new(LifecycleEmailEffect {
        app: ctx.app.clone(),
        app_id: ctx.app_id,
        provider: ctx.provider.to_string(),
        provider_webhook_id: ctx.webhook.provider_webhook_id.clone(),
        external_user_id: ctx.external_user_id.as_ref()?.clone(),
        subscription_id: subscription_id.to_string(),
        kind,
    })))
}

fn price_step_up_email_effect(ctx: &EventContext<'_>, subscription_id: &str) -> Option<PostCommitEffect> {
    let event_type = "subscription.price_step_up";
    let external_user_id_hash = ctx.external_user_id.as_deref().map(diagnostic_hash);
    let Some(new_price_cents) = ctx.fields.google_new_price_cents else {
        warn!(
            app_id = %ctx.app_id,
            provider = ctx.provider,
            event_type,
            provider_webhook_id = %ctx.webhook.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id,
            "Skipping price step-up email: missing new price"
        );
        return None;
    };
    let Some(deadline) = ctx.fields.google_price_step_up_consent_deadline.as_deref().and_then(parse_rfc3339_utc) else {
        warn!(
            app_id = %ctx.app_id,
            provider = ctx.provider,
            event_type,
            provider_webhook_id = %ctx.webhook.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id,
            "Skipping price step-up email: missing consent deadline"
        );
        return None;
    };

    lifecycle_email_effect(
        ctx,
        subscription_id,
        LifecycleEmailKind::PriceStepUp {
            new_price_cents,
            deadline,
        },
    )
}

fn payment_failed_email_effect(ctx: &EventContext<'_>, subscription_id: &str) -> Option<PostCommitEffect> {
    let event_type = "payment.failed";
    let external_user_id_hash = ctx.external_user_id.as_deref().map(diagnostic_hash);
    let Some(app_url) = ctx.app.app_url.clone() else {
        warn!(
            app_id = %ctx.app_id,
            provider = ctx.provider,
            event_type,
            provider_webhook_id = %ctx.webhook.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id,
            "Skipping payment failed email: app_url is not configured"
        );
        return None;
    };

    lifecycle_email_effect(ctx, subscription_id, LifecycleEmailKind::PaymentFailed { app_url })
}

pub(super) async fn handle_subscription_event<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    ctx: &EventContext<'_>,
) -> Result<EventHandling, BridgeError> {
    match ctx.canonical_event {
        "subscription.activated" | "subscription.renewed" | "subscription.recovered" | "subscription.created" => {
            let Some(user_id) = ctx.external_user_id.as_deref() else {
                return Ok(EventHandling::handled(EventEffects::default()));
            };

            let period_end = ctx.fields.current_period_end.as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let sub_id_fallback = ctx.webhook.subscription_id.clone().unwrap_or_default();
            let sub_id_str = ctx.fields.subscription_id.as_deref().unwrap_or(&sub_id_fallback);
            let Some(subscription_status) = activation_subscription_status(ctx.provider, ctx.fields.status.as_deref()) else {
                return Ok(EventHandling::ReturnNone);
            };

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

            let payment_provider_transaction_id = if ctx.provider == "creem" {
                ctx.fields.provider_transaction_id.as_deref()
            } else {
                Some(ctx.fields.provider_transaction_id.as_deref()
                    .unwrap_or(&ctx.webhook.provider_webhook_id))
            };
            let payment = payment_provider_transaction_id.map(|provider_transaction_id| {
                WebhookPaymentRecordRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    provider: ctx.provider,
                    provider_transaction_id,
                    provider_purchase_token: ctx.fields.purchase_token.as_deref().or(ctx.webhook.purchase_token.as_deref()),
                    ack_required: ctx.provider == "google_play"
                        && ctx.fields.purchase_token.as_deref().or(ctx.webhook.purchase_token.as_deref()).is_some(),
                    subscription_id: ctx.fields.subscription_id.as_deref(),
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(-1),
                    currency: ctx.fields.currency.as_deref().or(Some("UNKNOWN")),
                    status: "success",
                }
            });

            let subscription = repo
                .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    subscription_id: sub_id_str,
                    provider: ctx.provider,
                    status: &subscription_status,
                    current_period_end: period_end,
                    purchase_token: ctx.fields.purchase_token.as_deref(),
                    auto_renewing: ctx.fields.auto_renewing,
                    payment_state: None,
                    provider_customer_id: ctx.fields.provider_customer_id.as_deref(),
                    event_time_ms: ctx.timestamp_epoch_ms,
                    recurring_amount_cents: ctx.fields.amount_cents,
                    payment,
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

            if ctx.provider == "google_play" {
                let _ = repo
                    .link_replacement_subscriptions(ctx.app_id, user_id, sub_id_str, ctx.timestamp_epoch_ms)
                    .await;
            }

            Ok(EventHandling::handled(EventEffects {
                callback_event_type: Some("subscription.activated".to_string()),
                callback_status_override: Some(subscription_status),
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

            Ok(EventHandling::handled(EventEffects {
                should_forward: false,
                ..Default::default()
            }))
        }

        "subscription.trial_started" => {
            let Some(user_id) = ctx.external_user_id.as_deref() else {
                return Ok(EventHandling::handled(EventEffects::default()));
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
                    recurring_amount_cents: ctx.fields.amount_cents,
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

            Ok(EventHandling::handled(EventEffects {
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
                    return Ok(EventHandling::handled(EventEffects {
                        callback_event_type: Some("subscription.grace_period".to_string()),
                        callback_status_override: Some("past_due".to_string()),
                        canonical_subscription: Some(sub.into()),
                        ..Default::default()
                    }));
                }

                info!("Skipped stale grace_period event for subscription {}", sub_id);
                return Ok(EventHandling::ReturnNone);
            }

            Ok(EventHandling::handled(EventEffects::default()))
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

                return Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome)));
            }
            Ok(EventHandling::handled(EventEffects::default()))
        }

        "subscription.on_hold" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                if ctx.provider == "google_play" {
                    let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_on_hold(
                        repo,
                        ctx.app_id,
                        user_id,
                        ctx.webhook,
                        ctx.fields,
                        ctx.timestamp_epoch_ms,
                    ).await? else {
                        return Ok(EventHandling::ReturnNone);
                    };

                    return Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome)));
                } else {
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
                        return Ok(EventHandling::handled(EventEffects {
                            callback_event_type: Some("subscription.on_hold".to_string()),
                            callback_status_override: Some("on_hold".to_string()),
                            canonical_subscription: Some(sub.into()),
                            ..Default::default()
                        }));
                    }

                    info!("Skipped stale on_hold event for subscription {}", sub_id);
                    return Ok(EventHandling::ReturnNone);
                }
            }

            Ok(EventHandling::handled(EventEffects::default()))
        }

        "subscription.paused" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                if ctx.provider == "google_play" {
                    let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_paused(
                        repo,
                        ctx.app_id,
                        user_id,
                        ctx.webhook,
                        ctx.fields,
                        ctx.timestamp_epoch_ms,
                    ).await? else {
                        return Ok(EventHandling::ReturnNone);
                    };

                    let sub_id = ctx.fields.subscription_id.as_deref()
                        .or(ctx.webhook.subscription_id.as_deref())
                        .unwrap_or("");
                    let mut effects = effects_from_google_lifecycle_outcome(outcome);
                    effects.post_commit = lifecycle_email_effect(ctx, sub_id, LifecycleEmailKind::Paused)
                        .into_iter()
                        .collect();
                    return Ok(EventHandling::handled(effects));
                } else {
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
                                let post_commit = lifecycle_email_effect(ctx, &sub_id, LifecycleEmailKind::Paused)
                                    .into_iter()
                                    .collect();
                                return Ok(EventHandling::handled(EventEffects {
                                    callback_event_type: Some("subscription.paused".to_string()),
                                    callback_status_override: Some("paused".to_string()),
                                    canonical_subscription: Some(updated_sub.into()),
                                    post_commit,
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
            }

            Ok(EventHandling::handled(EventEffects::default()))
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

                if let Some(outcome) = outcome {
                    match ctx.fields.subscription_id.as_deref().or(ctx.webhook.subscription_id.as_deref()) {
                        Some(sub_id) => {
                        let period_end = outcome
                            .canonical_subscription
                            .as_ref()
                            .and_then(|s| s.current_period_end)
                            .unwrap_or_else(chrono::Utc::now);
                        let mut effects = effects_from_google_lifecycle_outcome(outcome);
                        if let Some(effect) = lifecycle_email_effect(
                            ctx,
                            sub_id,
                            LifecycleEmailKind::Resumed {
                                current_period_end: period_end,
                            },
                        ) {
                            effects.post_commit.push(effect);
                        }
                        return Ok(EventHandling::handled(effects));
                        }
                        None => return Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome))),
                    }
                }

                return Ok(EventHandling::handled(EventEffects::default()));
            }

            Ok(EventHandling::handled(EventEffects::default()))
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

                return Ok(EventHandling::handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()));
            }

            Ok(EventHandling::handled(EventEffects::default()))
        }

        "subscription.expired" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                if ctx.provider == "google_play" {
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
                                return Ok(EventHandling::handled(EventEffects {
                                    callback_event_type: Some("subscription.expired".to_string()),
                                    callback_status_override: Some("expired".to_string()),
                                    canonical_subscription: Some(updated_sub.into()),
                                    ..Default::default()
                                }));
                            }
                        } else {
                            info!("Google Play expired event skipped: purchase_token not found in database");
                            return Ok(EventHandling::ReturnNone);
                        }
                    } else {
                        info!("Google Play expired event skipped: missing purchase_token");
                        return Ok(EventHandling::ReturnNone);
                    }
                } else if let Some(purchase_token) = ctx.fields.purchase_token.as_deref() {
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
                            return Ok(EventHandling::handled(EventEffects {
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
                            return Ok(EventHandling::handled(EventEffects {
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
                        return Ok(EventHandling::handled(EventEffects {
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

            Ok(EventHandling::handled(EventEffects::default()))
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

                return Ok(EventHandling::handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()));
            }

            Ok(EventHandling::handled(EventEffects::default()))
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

                return Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome)));
            }

            Ok(EventHandling::handled(EventEffects::default()))
        }

        "subscription.updated" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                // An update event with no status field carries no lifecycle
                // signal. Defaulting it to "pending" would clobber the existing
                // row and expose it to delete_orphaned_pending_subscriptions, so
                // skip a missing status the same way we skip an unknown one.
                let Some(raw_status) = ctx.fields.status.as_deref() else {
                    return Ok(EventHandling::ReturnNone);
                };
                let adapter = ProviderWebhookAdapter::from_provider(ctx.provider)?;
                let Some(status) = adapter.normalize_status(Some(raw_status)).known() else {
                    return Ok(EventHandling::ReturnNone);
                };
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
                        recurring_amount_cents: ctx.fields.amount_cents,
                        payment: None,
                        adopt_stale_payment: false,
                        stale_payment_window_secs: 86400,
                    })
                    .await?;

                let Some(subscription) = subscription else {
                    return Ok(EventHandling::ReturnNone);
                };

                return Ok(EventHandling::handled(EventEffects {
                    callback_event_type: status_to_canonical_event(&status),
                    callback_status_override: Some(status),
                    canonical_subscription: Some(subscription),
                    ..Default::default()
                }));
            }

            Ok(EventHandling::handled(EventEffects::default()))
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

                if let Some(outcome) = outcome {
                    match ctx.fields.subscription_id.as_deref().or(ctx.webhook.subscription_id.as_deref()) {
                        Some(sub_id) => {
                        let mut effects = effects_from_google_lifecycle_outcome(outcome);
                        if let Some(effect) = price_step_up_email_effect(ctx, sub_id) {
                            effects.post_commit.push(effect);
                        }
                        return Ok(EventHandling::handled(effects));
                        }
                        None => return Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome))),
                    }
                }

                return Ok(EventHandling::handled(EventEffects::default()));
            }

            Ok(EventHandling::handled(EventEffects::default()))
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
                            return Ok(EventHandling::handled(EventEffects {
                                callback_status_override: Some("active".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                ..Default::default()
                            }));
                        }

                        info!("Skipped stale pause_scheduled event for subscription {}", sub_id);
                    }
                }
            }

            Ok(EventHandling::handled(EventEffects::default()))
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
                            let post_commit = lifecycle_email_effect(
                                ctx,
                                sub_id,
                                LifecycleEmailKind::Deferred {
                                    deferred_until: until,
                                },
                            )
                            .into_iter()
                            .collect();
                            return Ok(EventHandling::handled(EventEffects {
                                canonical_subscription: Some(updated_sub.into()),
                                post_commit,
                                ..Default::default()
                            }));
                        } else {
                            info!("Skipped stale deferred event for subscription {}", sub_id);
                        }
                    }
                }
            }

            Ok(EventHandling::handled(EventEffects::default()))
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
                        provider_purchase_token: None,
                        ack_required: false,
                        subscription_id: sub_id,
                        product_id: ctx.fields.product_id.as_deref(),
                        amount_cents: ctx.fields.amount_cents.unwrap_or(-1),
                        currency: currency.or(Some("UNKNOWN")),
                        status: "price_changed",
                    })
                    .await;
            }

            Ok(EventHandling::handled(EventEffects::default()))
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
                    return Ok(EventHandling::handled(EventEffects {
                        callback_event_type: Some("subscription.price_change_updated".to_string()),
                        callback_status_override: Some(updated_sub.status.clone()),
                        canonical_subscription: Some(updated_sub.into()),
                        ..Default::default()
                    }));
                }

                info!("Skipped stale price_change_updated event for subscription {}", sub_id);
                return Ok(EventHandling::ReturnNone);
            }

            Ok(EventHandling::handled(EventEffects::default()))
        }

        "subscription.expired_voided" => {
            info!(
                "Processed informational webhook event: {} (provider: {})",
                ctx.canonical_event,
                ctx.webhook.provider
            );
            Ok(EventHandling::handled(EventEffects::default()))
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
                    provider_purchase_token: ctx.fields.purchase_token.as_deref().or(ctx.webhook.purchase_token.as_deref()),
                    ack_required: false,
                    subscription_id: ctx.fields.subscription_id.as_deref(),
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(-1),
                    currency: ctx.fields.currency.as_deref().or(Some("UNKNOWN")),
                    status: "pending",
                })
                .await?;
            }

            Ok(EventHandling::handled(EventEffects {
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
                    provider_purchase_token: ctx.fields.purchase_token.as_deref().or(ctx.webhook.purchase_token.as_deref()),
                    ack_required: false,
                    subscription_id: if sub_id.is_empty() { None } else { Some(sub_id) },
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(-1),
                    currency: ctx.fields.currency.as_deref().or(Some("UNKNOWN")),
                    status: "failed",
                })
                .await?;

                if sub_id.is_empty() {
                    return Ok(EventHandling::handled(EventEffects {
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
                    return Ok(EventHandling::handled(EventEffects {
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
                    return Ok(EventHandling::handled(EventEffects {
                        callback_event_type: Some("payment.failed".to_string()),
                        callback_status_override: Some("failed".to_string()),
                        ..Default::default()
                    }));
                };

                let post_commit = payment_failed_email_effect(ctx, sub_id)
                    .into_iter()
                    .collect();

                return Ok(EventHandling::handled(EventEffects {
                    callback_event_type: Some("payment.failed".to_string()),
                    callback_status_override: Some("failed".to_string()),
                    canonical_subscription: Some(updated_sub.into()),
                    post_commit,
                    ..Default::default()
                }));
            }

            Ok(EventHandling::handled(EventEffects {
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

            Ok(EventHandling::handled(outcome.map(effects_from_google_lifecycle_outcome).unwrap_or_default()))
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

            Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome)))
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

            Ok(EventHandling::handled(effects_from_google_lifecycle_outcome(outcome)))
        }

        "payment.refunded" => {
            if let Some(_user_id) = ctx.external_user_id.as_deref() {
                let payment_tokens = [
                    ctx.fields.provider_transaction_id.as_deref(),
                    ctx.fields.purchase_token.as_deref(),
                    ctx.webhook.purchase_token.as_deref(),
                ];
                let mut matched_payment_token = None;
                let mut matched_payment_subscription_id = None;
                for token in payment_tokens.into_iter().flatten() {
                    if let Some(existing) = repo.get_payment_status_for_provider(ctx.app_id, ctx.provider, token).await? {
                        if existing != "refunded" {
                            repo.update_payment_status_for_provider(ctx.app_id, ctx.provider, token, "refunded").await?;
                        }
                        matched_payment_subscription_id = repo
                            .get_payment_subscription_id_for_provider(ctx.app_id, ctx.provider, token)
                            .await?;
                        matched_payment_token = Some(token);
                        break;
                    }
                }

                let subscription_tokens = [
                    ctx.fields.purchase_token.as_deref(),
                    ctx.webhook.purchase_token.as_deref(),
                    matched_payment_token,
                ];
                for token in subscription_tokens.into_iter().flatten() {
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
                            let post_commit = lifecycle_email_effect(
                                ctx,
                                &sub.subscription_id,
                                LifecycleEmailKind::Refunded,
                            )
                            .into_iter()
                            .collect();
                            return Ok(EventHandling::handled(EventEffects {
                                callback_event_type: Some("payment.refunded".to_string()),
                                callback_status_override: Some("refunded".to_string()),
                                canonical_subscription: Some(updated_sub.into()),
                                post_commit,
                                ..Default::default()
                            }));
                        }
                    }
                }

                if ctx.provider != "google_play" {
                    let subscription_ids = [
                        matched_payment_subscription_id.as_deref(),
                        ctx.fields.subscription_id.as_deref(),
                        ctx.webhook.subscription_id.as_deref(),
                    ];
                    for sub_id in subscription_ids.into_iter().flatten() {
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
                            let post_commit = lifecycle_email_effect(ctx, sub_id, LifecycleEmailKind::Refunded)
                                .into_iter()
                                .collect();
                            return Ok(EventHandling::handled(EventEffects {
                                callback_event_type: Some("payment.refunded".to_string()),
                                callback_status_override: Some("refunded".to_string()),
                                canonical_subscription: Some(_updated_sub.into()),
                                post_commit,
                                ..Default::default()
                            }));
                        }
                    }
                }
            }

            Ok(EventHandling::handled(EventEffects {
                callback_event_type: Some("payment.refunded".to_string()),
                callback_status_override: Some("refunded".to_string()),
                ..Default::default()
            }))
        }

        "payment.partially_refunded" => {
            if let Some(_user_id) = ctx.external_user_id.as_deref() {
                let payment_tokens = [
                    ctx.fields.provider_transaction_id.as_deref(),
                    ctx.fields.purchase_token.as_deref(),
                    ctx.webhook.purchase_token.as_deref(),
                ];
                for token in payment_tokens.into_iter().flatten() {
                    if let Some(existing) = repo.get_payment_status_for_provider(ctx.app_id, ctx.provider, token).await? {
                        if !matches!(existing.as_str(), "partially_refunded" | "refunded") {
                            repo.update_payment_status_for_provider(ctx.app_id, ctx.provider, token, "partially_refunded").await?;
                        }
                        break;
                    }
                }
            }

            Ok(EventHandling::handled(EventEffects {
                callback_event_type: Some("payment.partially_refunded".to_string()),
                callback_status_override: Some("partially_refunded".to_string()),
                ..Default::default()
            }))
        }

        "dispute.created" => {
            if let Some(user_id) = ctx.external_user_id.as_deref() {
                let txn_id = ctx.fields.provider_transaction_id.as_deref()
                    .or(ctx.webhook.subscription_id.as_deref())
                    .unwrap_or(&ctx.webhook.provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id: ctx.app_id,
                    external_user_id: user_id,
                    provider: ctx.provider,
                    provider_transaction_id: txn_id,
                    provider_purchase_token: None,
                    ack_required: false,
                    subscription_id: ctx.fields.subscription_id.as_deref(),
                    product_id: ctx.fields.product_id.as_deref(),
                    amount_cents: ctx.fields.amount_cents.unwrap_or(-1),
                    currency: ctx.fields.currency.as_deref().or(Some("UNKNOWN")),
                    status: "dispute_created",
                })
                .await?;
            }

            Ok(EventHandling::handled(EventEffects {
                callback_event_type: Some("dispute.created".to_string()),
                post_commit: vec![PostCommitEffect::DisputeAdminAlert(Box::new(DisputeAdminAlertEffect {
                    app: ctx.app.clone(),
                    webhook: ctx.webhook.clone(),
                    fields: ctx.fields.clone(),
                    external_user_id: ctx.external_user_id.clone(),
                }))],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app_snapshot(app_url: Option<String>) -> AppSnapshot {
        AppSnapshot {
            id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            slug: "test-app".to_string(),
            display_name: "Test App".to_string(),
            webhook_callback_url: "https://app.example/webhook".to_string(),
            webhook_callback_secret: "secret".to_string(),
            api_rate_limit_per_minute: 120,
            api_rate_limit_rules: None,
            app_url,
            google_package_name: None,
            apple_bundle_id: None,
        }
    }

    fn webhook_snapshot() -> WebhookProviderSnapshot {
        WebhookProviderSnapshot {
            provider: "google_play".to_string(),
            provider_webhook_id: "provider_evt_1".to_string(),
            event_type: "SUBSCRIPTION_ON_HOLD".to_string(),
            subscription_id: Some("sub_1".to_string()),
            purchase_token: Some("token_1".to_string()),
            payload: serde_json::json!({}),
            processed: false,
            timestamp_epoch_ms: Some(1770000000000),
            suppressed: false,
            suppressed_reason: None,
        }
    }

    #[test]
    fn activation_status_does_not_turn_unknown_creem_status_active() {
        assert_eq!(
            activation_subscription_status("creem", Some("paid")),
            Some("active".to_string())
        );
        assert_eq!(activation_subscription_status("creem", None), Some("active".to_string()));
        assert_eq!(activation_subscription_status("creem", Some("unpaid")), None);
        assert_eq!(activation_subscription_status("google_play", Some("ignored")), Some("active".to_string()));
    }

    #[test]
    fn payment_failed_email_effect_is_post_commit_data_with_stable_key() {
        let app = app_snapshot(Some("https://app.example".to_string()));
        let webhook = webhook_snapshot();
        let fields = WebhookFields::default();
        let external_user_id = Some("user_1".to_string());
        let ctx = EventContext {
            app: &app,
            app_id: app.id,
            canonical_event: "payment.failed",
            provider: "google_play",
            webhook: &webhook,
            fields: &fields,
            external_user_id: &external_user_id,
            timestamp_epoch_ms: 1770000000000,
        };

        let effect = payment_failed_email_effect(&ctx, "sub_1").unwrap();

        let PostCommitEffect::LifecycleEmail(effect) = effect else {
            panic!("expected lifecycle email effect");
        };
        assert_eq!(
            effect.idempotency_key(),
            "11111111-1111-1111-1111-111111111111:google_play:provider_evt_1:payment.failed:sub_1"
        );
        assert_eq!(effect.kind.event_type(), "payment.failed");
        assert_eq!(effect.external_user_id, "user_1");
        assert_eq!(effect.subscription_id, "sub_1");
        match effect.kind {
            LifecycleEmailKind::PaymentFailed { app_url } => {
                assert_eq!(app_url, "https://app.example");
            }
            _ => panic!("expected payment failed email effect"),
        }
    }

    #[test]
    fn payment_failed_email_effect_requires_app_url() {
        let app = app_snapshot(None);
        let webhook = webhook_snapshot();
        let fields = WebhookFields::default();
        let external_user_id = Some("user_1".to_string());
        let ctx = EventContext {
            app: &app,
            app_id: app.id,
            canonical_event: "payment.failed",
            provider: "google_play",
            webhook: &webhook,
            fields: &fields,
            external_user_id: &external_user_id,
            timestamp_epoch_ms: 1770000000000,
        };

        assert!(payment_failed_email_effect(&ctx, "sub_1").is_none());
    }
}

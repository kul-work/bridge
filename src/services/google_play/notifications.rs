#![allow(dead_code)]

/// Google Play notification and email helpers
use crate::application::checkout_helpers::format_cents_as_dollars;
use crate::error::BridgeError;
use crate::services::email::{EmailContext, EmailService};
use chrono::DateTime;
use chrono::Utc;

/// Send revocation notification to user
pub async fn send_email_revoked(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    reason: &str,
) -> Result<(), BridgeError> {
    email_service.send_email(
        email,
        "Subscription Revoked",
        &format!("Your subscription {} has been revoked. Reason: {}", subscription_id, reason),
    ).await
}

/// Send subscription restart confirmation to user
pub async fn send_email_restarted(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    current_period_end: DateTime<Utc>,
    context: EmailContext<'_>,
) -> Result<(), BridgeError> {
    email_service.send_email_with_context(
        email,
        "Subscription Restarted",
        &format!("Your subscription {} has been restarted. Current period ends: {}", subscription_id, current_period_end),
        context,
    ).await
}

/// Send subscription cancellation notification to user
pub async fn send_email_cancelled(
    email_service: &dyn EmailService,
    email: &str, 
    subscription_id: &str
) -> Result<(), BridgeError> {
    email_service.send_email(
        email,
        "Subscription Cancelled",
        &format!("Your subscription {} has been cancelled.", subscription_id),
    ).await
}

/// Send scheduled cancellation warning to user
pub async fn send_email_cancellation_scheduled(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    cancellation_deadline: Option<DateTime<Utc>>,
) -> Result<(), BridgeError> {
    email_service.send_email(
        email,
        "Subscription Cancellation Scheduled",
        &format!("Your subscription {} is scheduled for cancellation. Deadline: {:?}", subscription_id, cancellation_deadline),
    ).await
}

/// Send price increase notification to user
pub async fn send_email_price_step_up(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    new_price_cents: i64,
    deadline: DateTime<Utc>,
    context: EmailContext<'_>,
) -> Result<(), BridgeError> {
    email_service.send_email_with_context(
        email,
        "Subscription Price Increase",
        &format!("Your subscription {} price is increasing to ${} effective {}.", subscription_id, format_cents_as_dollars(new_price_cents), deadline),
        context,
    ).await
}

/// Send price increase rejection notification to user
pub async fn send_email_price_step_up_rejected(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
) -> Result<(), BridgeError> {
    email_service.send_email(
        email,
        "Price Increase Declined",
        &format!("You have declined the price increase for subscription {}.", subscription_id),
    ).await
}

/// Send deferred renewal notification to user
pub async fn send_email_deferred(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    deferred_until: DateTime<Utc>,
    context: EmailContext<'_>,
) -> Result<(), BridgeError> {
    email_service.send_email_with_context(
        email,
        "Subscription Renewal Deferred",
        &format!("Your subscription {} renewal is deferred until {}.", subscription_id, deferred_until),
        context,
    ).await
}

/// Send subscription pause notification to user
pub async fn send_email_paused(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    context: EmailContext<'_>,
) -> Result<(), BridgeError> {
    email_service.send_email_with_context(
        email,
        "Subscription Paused",
        &format!("Your subscription {} has been paused. You can resume it at any time from your account settings.", subscription_id),
        context,
    ).await
}

/// Send subscription refund notification to user
pub async fn send_email_refunded(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    context: EmailContext<'_>,
) -> Result<(), BridgeError> {
    email_service.send_email_with_context(
        email,
        "Refund Processed",
        &format!("A refund has been processed for your subscription {}. Your subscription has been revoked as a result.", subscription_id),
        context,
    ).await
}

/// Send payment failure notification to user with actionable links
pub async fn send_email_payment_failed(
    email_service: &dyn EmailService,
    email: &str,
    subscription_id: &str,
    provider_name: &str,
    app_url: &str,
    context: EmailContext<'_>,
) -> Result<(), BridgeError> {
    
    let provider_display = match provider_name {
        "google_play" => "Google Play",
        "creem" => "Creem",
        _ => provider_name,
    };

    // Build actionable links
    let billing_link = format!(
        "{}/billing/payment-method?subscription_id={}",
        app_url.trim_end_matches('/'),
        subscription_id
    );

    // Provider-specific subscription management link
    let provider_link = match provider_name {
        "google_play" => Some("https://play.google.com/store/account/subscriptions".to_string()),
        "creem" => None, // Creem manages via portal
        _ => None,
    };

    // Build email body with actionable links
    let mut body = format!(
        "Your payment for subscription {} via {} has failed.\n\n\
        Please update your payment method to continue your service:\n\
        {}\n",
        subscription_id,
        provider_display,
        billing_link
    );

    if let Some(link) = provider_link {
        body.push_str(&format!(
            "\nOr manage your subscription directly:\n{}\n",
            link
        ));
    }

    body.push_str(
        "\nIf you don't update your payment method, your subscription may be cancelled."
    );

    email_service.send_email_with_context(
        email,
        "Payment Failed - Action Required",
        &body,
        context,
    ).await
}

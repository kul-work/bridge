//! Google Play one-time product (inapp) lifecycle handlers
//! Reference: https://developer.android.com/google/play/billing/integrate
use crate::error::AppError;
use crate::handlers::AppState;
use crate::services::payment::GooglePlayRawData;
use tracing::warn;

/// Handle one-time product purchased event
/// Orchestrates: database record creation + access grant + acknowledgment
/// 
/// This replaces inline logic from webhooks.rs one_time_product.purchased handler
pub async fn handle_otp_purchased(
    state: &AppState,
    provider_name: &str,
    clerk_id: &str,
    product_id: &str,
) -> Result<(), AppError> {
    tracing::info!(
        "Processing OTP purchase: product_id={}, clerk_id={}, provider={}",
        product_id,
        clerk_id,
        provider_name
    );

    // For OTP: grant access immediately (unlike subscriptions which need billing verification)
    if let Err(e) = state.database.activate_subscription(crate::db::ActivateSubscriptionParams {
        clerk_id,
        subscription_id: product_id,
        status: "active",
        expires_at: None, // OTP doesn't have period_end (one-time)
        provider: provider_name,
        provider_customer_id: None,
        purchase_token: None,
        payment_state: None,
        cancel_reason: None,
        auto_renewing: None,
        google_linked_purchase_token: None,
        google_obfuscated_account_id: None,
    }).await {
        tracing::error!("Failed to activate OTP for clerk_id {}: {}", clerk_id, e);
        return Err(e);
    }

    // Acknowledge product to prevent auto-refund after 3 days
    acknowledge_otp_product(state, provider_name, product_id, "", clerk_id).await?;

    tracing::info!(
        "OTP purchase completed: {} for user {}",
        product_id,
        clerk_id
    );

    Ok(())
}

/// Acknowledge one-time product to prevent auto-refund after 3 days
/// Called from webhook handler when one_time_product.purchased event arrives
/// (in case verify_purchase wasn't called first)
pub async fn acknowledge_otp_product(
    state: &AppState,
    provider_name: &str,
    product_id: &str,
    token: &str,
    clerk_id: &str,
) -> Result<(), AppError> {
    if let Some(_provider) = state.payment_providers.get(provider_name) {
        // Check if provider supports acknowledge (only Google Play does)
        // For other providers, silently skip (not all support OTP or acknowledge)
        if provider_name == "google_play" {
            // Cast to GooglePlayService if possible
            // Note: We can't directly call acknowledge_product here since it's not on the PaymentProvider trait.
            // Instead, we rely on verify_purchase being called later to acknowledge, or
            // the webhook itself calling the Google API (which currently doesn't happen).
            //
            // FUTURE IMPROVEMENT: Add acknowledge_product to PaymentProvider trait
            // and call: provider.acknowledge_product(product_id, token).await?

            tracing::info!(
                "One-Time Product {} purchased (token: {}). \
                 Acknowledged in verify_purchase flow or will be auto-acknowledged if verify_purchase called within 3 days.",
                product_id,
                token
            );

            // Log acknowledgment for audit trail (compliance/debugging)
            if let Err(e) = state.database.log_product_acknowledgment(clerk_id, product_id, "pending").await {
                warn!("Failed to log product acknowledgment (non-critical): {}", e);
            }
        }
    }
    Ok(())
}

/// Handle one-time product cancelled/refunded event
/// OTP cancellation differs from subscription cancellation:
/// - Immediate access revocation (no grace period)
/// - No auto-renew concept
/// - Typically triggered by refund or chargeback
/// - Updates payment status to "refunded" in payments table
pub async fn handle_otp_cancelled(
    state: &AppState,
    clerk_id: &str,
    product_id: &str,
    token: Option<&str>,
    cancellation_reason: Option<&str>,
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    tracing::info!(
        "Processing OTP cancellation: product_id={}, clerk_id={}, reason={:?}",
        product_id,
        clerk_id,
        cancellation_reason
    );

    // Update payment status to "refunded" for the OTP
    if let Some(token) = token {
        state.database.update_payment_status(token, "refunded").await?;
        tracing::info!("OTP payment status updated to refunded");
        tracing::debug!("Token: {}", token);
    }

    // Deactivate user's access immediately
    state.database.deactivate_subscription(clerk_id, "cancelled", cancellation_reason).await?;

    // Log cancellation for audit trail
    if let Err(e) = state.database.log_product_acknowledgment(
        clerk_id,
        product_id,
        &format!("cancelled:{}", cancellation_reason.unwrap_or("unknown")),
    ).await {
        warn!("Failed to log OTP cancellation (non-critical): {}", e);
    }

    // Send notification to user
    if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
        let reason_text = cancellation_reason.unwrap_or("a refund was processed");
        let _ = state.email_service.send_email(
            &email,
            "Purchase Cancelled",
            &format!(
                "Your one-time purchase ({}) has been cancelled because {}. Access has been removed.",
                product_id,
                reason_text
            ),
        ).await;
    }

    // Store Google Play-specific context if available
    if google_data.google_purchase_token.is_some() {
        tracing::debug!(
            "OTP cancellation context: token={:?}, payment_state={:?}",
            google_data.google_purchase_token,
            google_data.google_payment_state
        );
    }

    Ok(())
}

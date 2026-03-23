//! Google Play subscription lifecycle handlers
//! Reference: https://developer.android.com/google/play/billing/integrate
use crate::error::AppError;
use crate::handlers::AppState;
use crate::services::payment::{
    GooglePlayRawData, NormalizedSubscriptionData, SubscriptionRecord, SubscriptionStatus, PurchaseType,
};
use chrono::Utc;
use tracing::{info, warn};

use super::notifications::*;

/// Account Hold Handler
/// User loses access immediately when account hold is triggered
/// This follows the same pattern as other subscription state handlers
pub async fn handle_subscription_on_hold(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    event_time_millis: i64,
) -> Result<(), AppError> {
    info!(
        "Processing subscription on hold: {} for user {} (event_time: {})",
        subscription_id, clerk_id, event_time_millis
    );

    // Use atomic transaction to ensure subscriptions + users both updated
    let mut tx = state.database.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction for account hold: {}", e);
        e
    })?;

    let changed = state
        .database
        .handle_account_hold_tx(&mut tx, clerk_id, subscription_id, event_time_millis)
        .await
        .map_err(|e| {
        tracing::error!("Failed to handle account hold: {}", e);
        e
    })?;

    tx.commit().await?;

    if !changed {
        tracing::debug!(
            "Skipped account hold for subscription {} (stale event or no-op)",
            subscription_id
        );
        return Ok(());
    }

    // Send notification to user about account hold
    if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
        let message = format!(
            "Your subscription has been placed on hold due to a billing issue. \
            Please update your payment method to restore access. Subscription: {}",
            subscription_id
        );
        crate::services::email::send_email_mock(
            &email,
            "Account Hold - Action Required",
            &message,
        );
    }

    tracing::info!("Subscription on hold handler completed for {}", subscription_id);

    Ok(())
}

/// SUBSCRIPTION_REVOKED Status Handler
/// Distinguish revoked subscriptions (refund/chargeback) from natural expiration
/// Store revocation reason for immediate access loss
pub async fn handle_subscription_revoked(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    revocation_reason: &str, // "REFUND", "CHARGEBACK", "FRAUD", "ACCOUNT_HOLD", "DEVELOPER_REVOKED"
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing revoked subscription: {} (reason: {})",
        subscription_id, revocation_reason
    );

    // Create normalized subscription with REVOKED status
    let normalized = NormalizedSubscriptionData {
        status: SubscriptionStatus::Revoked,
        current_period_end: None,
        auto_renewing: Some(false),
        cancellation_initiated_at: None,
        revocation_reason: Some(revocation_reason.to_string()),
        revoked_at: Some(Utc::now()),
    };

    // Store revocation with reason for analytics/support
    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized,
        google_data: Some(google_data.clone()),
        last_event_time: google_data
            .google_event_time_millis
            .unwrap_or_else(|| Utc::now().timestamp_millis()),
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Immediately deactivate user's premium access (no grace period for revocation)
        state.database.deactivate_subscription(clerk_id, "revoked", Some(revocation_reason)).await?;

        // Notify user
        if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
            let reason_text = match revocation_reason {
                "REFUND" => "a refund was issued",
                "CHARGEBACK" => "a chargeback dispute was filed",
                "FRAUD" => "fraudulent activity was detected",
                "ACCOUNT_HOLD" => "your account is on hold",
                "DEVELOPER_REVOKED" => "your subscription was revoked",
                _ => "your subscription was revoked due to a billing issue",
            };
            send_email_revoked(state.email_service.as_ref(), &email, subscription_id, reason_text).await?;
        }
    } else {
        tracing::debug!(
            "Skipped revoked side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// Resubscribe & Upgrade Account Linking
/// 
/// For resubscribe: Extract expired external account identifiers and link to previous subscription
/// For upgrades/downgrades: Mark the previous active subscription as "replaced"
/// 
/// Google Play doesn't send linkedPurchaseToken in webhooks, so we detect upgrades by:
/// 1. Checking if user already has an active subscription for this subscription_id
/// 2. If yes, mark the old one as "replaced"
pub async fn handle_resubscribe_linking(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    purchase_token: &str,
    obfuscated_account_id: Option<&str>,
    _linked_purchase_token: Option<&str>, // Kept for signature compat; Google doesn't send this
) -> Result<(), AppError> {
    info!(
        "Processing subscription activation for user {}: subscription {}",
        clerk_id, subscription_id
    );

    // UPGRADE/DOWNGRADE DETECTION
    // Check if there's already an active subscription for this user
    match state.database.get_active_subscription(clerk_id).await {
        Ok(Some((existing_sub_id, _provider))) => {
            // User has an existing active subscription
            if existing_sub_id != subscription_id {
                // Different subscription → this is an upgrade/downgrade
                info!(
                    "Upgrade/downgrade detected for user {}: {} → {}",
                    clerk_id, existing_sub_id, subscription_id
                );
                
                // Mark old subscription as replaced (inactive)
                if let Err(e) = state
                    .database
                    .mark_subscription_replaced(&existing_sub_id, clerk_id)
                    .await
                {
                    warn!(
                        "Failed to mark {} as replaced: {}",
                        existing_sub_id, e
                    );
                    // Non-fatal, continue processing
                }
            }
        }
        Ok(None) => {
            // No existing active subscription (first purchase or resubscribe after expiry)
            info!(
                "No existing active subscription for user {} - this is a new or resubscribe",
                clerk_id
            );
        }
        Err(e) => {
            warn!("Failed to check for existing subscription: {}", e);
            // Non-fatal, continue processing
        }
    }

    // Send acknowledgement with account IDs
    if let Some(account_id) = obfuscated_account_id {
        acknowledge_subscription_with_account_id(
            state,
            subscription_id,
            purchase_token,
            account_id,
        )
        .await?;
    }

    Ok(())
}

/// SUBSCRIPTION_RESTARTED Webhook Handler
/// User re-enabled auto-renew after cancellation but before expiry.
/// Backend must update to ACTIVE status.
pub async fn handle_subscription_restarted(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    current_period_end: chrono::DateTime<Utc>,
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing subscription restart: {} (user re-enabled auto-renew)",
        subscription_id
    );

    let normalized = NormalizedSubscriptionData {
        status: SubscriptionStatus::Active,
        current_period_end: Some(current_period_end),
        auto_renewing: Some(true),
        cancellation_initiated_at: None, // Clear cancellation request
        revocation_reason: None,
        revoked_at: None,
    };

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized,
        google_data: Some(google_data.clone()),
        last_event_time: google_data
            .google_event_time_millis
            .unwrap_or_else(|| Utc::now().timestamp_millis()),
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Re-activate premium access
        state
            .database
            .activate_subscription(crate::db::ActivateSubscriptionParams {
                clerk_id,
                subscription_id,
                status: "active",
                expires_at: Some(current_period_end),
                provider: "google_play",
                provider_customer_id: None,
                purchase_token: google_data.google_purchase_token.as_deref(),
                payment_state: google_data.google_payment_state,
                cancel_reason: google_data.google_cancel_reason,
                auto_renewing: Some(true),
                google_linked_purchase_token: None,
                google_obfuscated_account_id: None,
            })
            .await?;

        // Notify user
        if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
            send_email_restarted(state.email_service.as_ref(), &email, subscription_id, current_period_end).await?;
        }
    } else {
        tracing::debug!(
            "Skipped restarted side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// Cancel Reason Context Persistence
/// Persist canceledStateContext and userInitiatedCancellation feedback for analytics
pub async fn handle_subscription_cancelled_with_context(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    cancellation_context: Option<&str>, // "USER_CANCELED", "SYSTEM_CANCELED", "DEVELOPER_INITIATED"
    cancellation_feedback: Option<&str>, // User-provided reason (optional)
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing subscription cancellation: {} (context: {:?})",
        subscription_id, cancellation_context
    );

    let normalized = NormalizedSubscriptionData {
        status: SubscriptionStatus::Cancelled,
        current_period_end: google_data.google_grace_period_end, // Can still use access until end of period
        auto_renewing: Some(false),
        cancellation_initiated_at: Some(Utc::now()),
        revocation_reason: None, // Only revoked = immediate loss of access
        revoked_at: None,
    };

    let mut google_data_updated = google_data.clone();
    google_data_updated.google_cancellation_context =
        cancellation_context.map(|s| s.to_string());
    google_data_updated.google_cancellation_feedback =
        cancellation_feedback.map(|s| s.to_string());

    info!(
        "Before store_subscription_record: cancellation_initiated_at={:?}, google_cancellation_context={:?}, google_cancellation_feedback={:?}",
        normalized.cancellation_initiated_at, google_data_updated.google_cancellation_context, google_data_updated.google_cancellation_feedback
    );
    let event_time = google_data_updated
        .google_event_time_millis
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized,
        google_data: Some(google_data_updated),
        last_event_time: event_time,
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Update user premium status: cancelled subscription grants access until period_end
        if let Some(period_end) = google_data.google_grace_period_end {
            state.database.update_premium_on_cancellation(clerk_id, period_end).await?;
        }

        // Log cancellation reason for analytics
        state
            .database
            .log_cancellation_reason(
                clerk_id,
                subscription_id,
                cancellation_context.unwrap_or("UNKNOWN"),
                cancellation_feedback,
            )
            .await?;

        // Notify user (access continues until current_period_end)
        if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
            send_email_cancelled(state.email_service.as_ref(), &email, subscription_id).await?;
        }
    } else {
        tracing::debug!(
            "Skipped cancelled side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

///  Handler subscription.cancellation_scheduled
/// User cancelled mid-commitment. Mark as pending_cancellation (will cancel after commitment ends).
pub async fn handle_subscription_cancellation_scheduled(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    cancellation_deadline: Option<chrono::DateTime<Utc>>,
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing scheduled cancellation: {} (will cancel at {:?})",
        subscription_id, cancellation_deadline
    );

    let mut google_data_updated = google_data.clone();
    google_data_updated.google_pending_cancellation = true;
    google_data_updated.google_pending_cancellation_at = Some(Utc::now());
    let event_time = google_data_updated
        .google_event_time_millis
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    // Status remains ACTIVE (user still has access until cancellation_deadline)
    let normalized = NormalizedSubscriptionData {
        status: SubscriptionStatus::Active,
        current_period_end: cancellation_deadline,
        auto_renewing: Some(false), // Auto-renew is OFF, will cancel
        cancellation_initiated_at: Some(Utc::now()),
        revocation_reason: None,
        revoked_at: None,
    };

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized,
        google_data: Some(google_data_updated),
        last_event_time: event_time,
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Notify user
        if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
            send_email_cancellation_scheduled(state.email_service.as_ref(), &email, subscription_id, cancellation_deadline).await?;
        }
    } else {
        tracing::debug!(
            "Skipped cancellation_scheduled side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// Price Step-Up Consent (Korea-specific)
/// Handle price increase consent request. Auto-cancel if not accepted before deadline.
pub async fn handle_price_step_up_consent_required(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    new_price_cents: i32,
    consent_deadline: chrono::DateTime<Utc>,
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing price step-up consent: {} (new price: ${} at {:?})",
        subscription_id,
        new_price_cents as f64 / 100.0,
        consent_deadline
    );

    let mut google_data_updated = google_data.clone();
    google_data_updated.google_requires_price_step_up_consent = true;
    google_data_updated.google_price_step_up_consent_status = Some("pending".to_string());
    google_data_updated.google_price_step_up_consent_deadline = Some(consent_deadline);
    google_data_updated.google_new_price_cents = Some(new_price_cents);
    let event_time = google_data_updated
        .google_event_time_millis
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized: NormalizedSubscriptionData::default(),
        google_data: Some(google_data_updated),
        last_event_time: event_time,
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Notify user of price change
        if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
            send_email_price_step_up(state.email_service.as_ref(), &email, subscription_id, new_price_cents, consent_deadline).await?;
        }

        // Schedule auto-cancel if not consented by deadline
        state
            .database
            .schedule_price_step_up_auto_cancel(clerk_id, subscription_id, consent_deadline)
            .await?;
    } else {
        tracing::debug!(
            "Skipped price_step_up_required side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// Handle price step-up consent response from Google Play
/// 
/// NOTE: This function handles webhook callbacks AFTER consent flow.
/// 
/// Backend Pieces:
/// - Detect price change requirement via webhook/API
/// - Fetch price change details from Google Play API
/// - Automatically notify user via email
/// - Handle consent acceptance/rejection and auto-cancel
///
/// Frontend Pieces:
/// - Frontend checks `google_requires_price_step_up_consent` flag
/// - Displays Bootstrap modal (PriceStepUpConsent.tsx) to user for explicit consent
/// - Auto-shown on app resume via appLifecycleService
pub async fn handle_price_step_up_consent_updated(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    consent_status: &str, // "accepted", "rejected", "timed_out"
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing price step-up consent update: {} (status: {})",
        subscription_id, consent_status
    );

    let mut google_data_updated = google_data.clone();
    google_data_updated.google_price_step_up_consent_status = Some(consent_status.to_string());
    let event_time = google_data_updated
        .google_event_time_millis
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized: NormalizedSubscriptionData::default(),
        google_data: Some(google_data_updated),
        last_event_time: event_time,
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        match consent_status {
            "accepted" => {
                info!("User accepted price step-up for {}", subscription_id);
                // Subscription will continue with new price
            }
            "rejected" | "timed_out" => {
                warn!(
                    "User rejected/timed out price step-up for {}. Auto-cancelling subscription.",
                    subscription_id
                );
                // Auto-cancel subscription
                state.database.deactivate_subscription(clerk_id, "cancelled", None).await?;
                if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
                    send_email_price_step_up_rejected(state.email_service.as_ref(), &email, subscription_id).await?;
                }
            }
            _ => {
                warn!(
                    "Unknown price step-up consent status: {} for {}",
                    consent_status, subscription_id
                );
            }
        }
    } else {
        tracing::debug!(
            "Skipped price_step_up_updated side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// Subscription State from API
/// Use subscription_state (0-6 enum) as primary source of truth.
/// This is called when parsing API responses; the mapping is done here.
/// Subscription State from API (V2)
/// Use subscription_state (String Enum) as primary source of truth.
/// This is called when parsing API responses; the mapping is done here.
pub fn map_google_subscription_state_to_normalized(
    google_state: Option<&str>,
    _payment_state: Option<i32>, // Deprecated V1, kept for interface or if used elsewhere
    _cancel_reason: Option<i32>,
    current_period_end: Option<chrono::DateTime<Utc>>,
    grace_period_end: Option<chrono::DateTime<Utc>>,
    deferred_until: Option<chrono::DateTime<Utc>>,
) -> NormalizedSubscriptionData {
    // subscription_state is String Enum in V2
    // SUBSCRIPTION_STATE_PENDING, SUBSCRIPTION_STATE_ACTIVE, SUBSCRIPTION_STATE_PAUSED,
    // SUBSCRIPTION_STATE_IN_GRACE_PERIOD, SUBSCRIPTION_STATE_ON_HOLD, SUBSCRIPTION_STATE_CANCELED,
    // SUBSCRIPTION_STATE_EXPIRED
    
    let status = match google_state {
        Some("SUBSCRIPTION_STATE_ACTIVE") => SubscriptionStatus::Active,
        Some("SUBSCRIPTION_STATE_CANCELED") => SubscriptionStatus::Cancelled,
        Some("SUBSCRIPTION_STATE_IN_GRACE_PERIOD") => SubscriptionStatus::PastDue,
        Some("SUBSCRIPTION_STATE_ON_HOLD") => SubscriptionStatus::OnHold,
        Some("SUBSCRIPTION_STATE_PAUSED") => SubscriptionStatus::Paused,
        Some("SUBSCRIPTION_STATE_PENDING") => SubscriptionStatus::Pending,
        Some("SUBSCRIPTION_STATE_EXPIRED") => SubscriptionStatus::Expired,
        _ => SubscriptionStatus::Expired, // Default to expired if unknown or missing (safer)
    };

    // Determine actual expiration considering deferred, grace period
    let period_end = deferred_until.or(grace_period_end).or(current_period_end);

    NormalizedSubscriptionData {
        status,
        current_period_end: period_end,
        auto_renewing: match google_state {
            Some("SUBSCRIPTION_STATE_CANCELED") => Some(false), // Cancelled = no auto-renew
            Some("SUBSCRIPTION_STATE_EXPIRED") => Some(false),
            _ => None,        // Determine from other sources or default
        },
        cancellation_initiated_at: if matches!(status, SubscriptionStatus::Cancelled) {
            Some(Utc::now())
        } else {
            None
        },
        revocation_reason: None, // Must be set separately if revoked
        revoked_at: None,
    }
}

/// Deferred Status Handling
/// Developer deferred renewal. User has access but not charged until deferred_until.
pub async fn handle_subscription_deferred(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    deferred_until: chrono::DateTime<Utc>,
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing deferred subscription: {} (will charge at {:?})",
        subscription_id, deferred_until
    );

    let mut google_data_updated = google_data.clone();
    google_data_updated.google_deferred_until = Some(deferred_until);
    let event_time = google_data_updated
        .google_event_time_millis
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    let normalized = NormalizedSubscriptionData {
        status: SubscriptionStatus::Active, // User retains access
        current_period_end: Some(deferred_until), // Don't charge until this date
        auto_renewing: Some(true),
        cancellation_initiated_at: None,
        revocation_reason: None,
        revoked_at: None,
    };

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized,
        google_data: Some(google_data_updated),
        last_event_time: event_time,
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Notify user of promotional extension
        if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
            send_email_deferred(state.email_service.as_ref(), &email, subscription_id, deferred_until).await?;
        }
    } else {
        tracing::debug!(
            "Skipped deferred side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// Pending Purchases Handling
/// SUBSCRIPTION_STATE_PENDING: wait for confirmation before granting access
pub async fn handle_subscription_pending(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    google_data: &GooglePlayRawData,
) -> Result<(), AppError> {
    info!(
        "Processing pending subscription: {} (awaiting confirmation)",
        subscription_id
    );

    let normalized = NormalizedSubscriptionData {
        status: SubscriptionStatus::Pending,
        current_period_end: None, // No access yet
        auto_renewing: None,
        cancellation_initiated_at: None,
        revocation_reason: None,
        revoked_at: None,
    };

    let record = SubscriptionRecord {
        clerk_id: clerk_id.to_string(),
        subscription_id: subscription_id.to_string(),
        provider: "google_play".to_string(),
        provider_customer_id: None,
        amount_cents: None,
        normalized,
        google_data: Some(google_data.clone()),
        last_event_time: google_data
            .google_event_time_millis
            .unwrap_or_else(|| Utc::now().timestamp_millis()),
    };

    let outcome = state.database.store_subscription_record(&record).await?;

    if matches!(outcome, crate::db::SubscriptionStoreOutcome::Applied) {
        // Mark user as not premium (no access until confirmed)
        state
            .database
            .deactivate_subscription(clerk_id, "pending", None)
            .await?;
    } else {
        tracing::debug!(
            "Skipped pending side effects for {} (stale event)",
            subscription_id
        );
    }

    Ok(())
}

/// HELPER FUNCTIONS
async fn acknowledge_subscription_with_account_id(
    state: &AppState,
    subscription_id: &str,
    purchase_token: &str,
    obfuscated_account_id: &str,
) -> Result<(), AppError> {
    // Get Google Play provider and acknowledge the subscription
    let provider = state
        .payment_providers
        .get("google_play")
        .ok_or_else(|| AppError::PaymentProviderError("Google Play provider not configured".to_string()))?;

    // Get clerk_id for DB updates
    let _clerk_id = state.database.get_clerk_id_by_subscription(subscription_id).await?
        .ok_or_else(|| AppError::SubscriptionNotFound)?;

    // Idempotency check: only acknowledge if this specific purchase token hasn't been acknowledged yet
    if !state.database.has_payment_been_acknowledged(purchase_token).await? {
        // Call acknowledge method (application layer enforces idempotency)
        provider.acknowledge_purchase_idempotent(
            subscription_id,
            purchase_token,
            PurchaseType::Subscription,
            Some(obfuscated_account_id),
        ).await?;
        
        // Mark payment as acknowledged to prevent re-acknowledgment on retry
        state.database.mark_payment_acknowledged(purchase_token).await?;
    } else {
        tracing::debug!(
            "Subscription {} already acknowledged (by purchase_token), skipping API call",
            subscription_id
        );
    }

    // Log the acknowledgement for audit trail (happens regardless of whether API was called)
    state
        .database
        .log_product_acknowledgment(
            obfuscated_account_id,
            subscription_id,
            "acknowledged_with_account_id",
        )
        .await?;

    info!(
        "Acknowledged subscription {} with account {} via Google Play API",
        subscription_id, obfuscated_account_id
    );

    Ok(())
}

/// SUBSCRIPTION_PAUSED Status Handler (Type 10)
/// User paused their subscription temporarily - they have no access while paused
/// Can be resumed later via Type 7 (RESTARTED)
pub async fn handle_subscription_paused(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    event_time_millis: Option<i64>,
) -> Result<(), AppError> {
    info!(
        "Processing paused subscription: {} for user {}",
        subscription_id, clerk_id
    );

    let event_time = event_time_millis.unwrap_or_else(|| Utc::now().timestamp_millis());

    // Use DB abstraction to handle pausing and revoking access atomically.
    // Must happen before writing the audit/event record, otherwise state guards
    // may skip the transition when status is already "paused".
    let changed = state
        .database
        .pause_subscription(clerk_id, subscription_id, event_time)
        .await?;
    if !changed {
        return Ok(());
    }

    // pause_subscription already performs the guarded state transition and updates
    // last_event_time atomically; a follow-up store_subscription_record with the
    // same event_time would be skipped by chronology and not add metadata.
    if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
        crate::services::email::send_email_mock(
            &email,
            "Subscription Paused",
            &format!(
                "Your subscription has been paused. \
                You will not be charged during the pause period. \
                You can resume anytime. Subscription: {}",
                subscription_id
            ),
        );
    }

    tracing::info!("Subscription paused handler completed for {}", subscription_id);

    Ok(())
}

/// SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED Handler (Type 11)
/// User scheduled a pause for a future date
/// Subscription remains ACTIVE until pause_date, then transitions to PAUSED
pub async fn handle_subscription_pause_scheduled(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    pause_scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), AppError> {
    info!(
        "Processing pause scheduled: {} for user {} (scheduled for: {:?})",
        subscription_id, clerk_id, pause_scheduled_at
    );

    // Update subscription with pause schedule metadata
    // Subscription remains ACTIVE until pause_scheduled_at arrives
    // Backend should have a job to monitor these and transition to PAUSED when date arrives
    state.database.schedule_subscription_pause(clerk_id, subscription_id, pause_scheduled_at).await?;

    // Notify user about scheduled pause
    if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
        let pause_date_str = pause_scheduled_at
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "a future date".to_string());
        
        crate::services::email::send_email_mock(
            &email,
            "Subscription Pause Scheduled",
            &format!(
                "You have scheduled a pause for your subscription effective {}. \
                You will continue to have access until that date, \
                then your subscription will be paused and you won't be charged. \
                Subscription: {}",
                pause_date_str,
                subscription_id
            ),
        );
    }

    tracing::info!("Pause scheduled handler completed for {}", subscription_id);

    Ok(())
}

/// SUBSCRIPTION_RECOVERED From Paused State Handler (Type 7)
/// Handles recovery from pause - distinguishes manual resume vs auto-resume
/// Called when user resumes their subscription or auto-resume triggers
/// Phase 2: Will be called from webhooks.rs when recovering from paused state
#[allow(dead_code)]
pub async fn handle_subscription_recovered_from_paused(
    state: &AppState,
    clerk_id: &str,
    subscription_id: &str,
    is_manual_resume: bool,
) -> Result<(), AppError> {
    info!(
        "Processing subscription recovery from paused: {} for user {} (manual: {})",
        subscription_id, clerk_id, is_manual_resume
    );

    // Check if subscription was actually in paused state
    let prev_status = state.database.get_subscription_status(clerk_id, subscription_id).await?;

    if prev_status != "paused" {
        warn!(
            "Recovery received for subscription {} but status is {}, not paused. Treating as normal recovery.",
            subscription_id,
            prev_status
        );
    }

    let event_time = Utc::now().timestamp_millis();
    let changed = state
        .database
        .resume_subscription(
            clerk_id,
            subscription_id,
            event_time,
            Some(is_manual_resume),
        )
        .await?;
    if !changed {
        tracing::debug!(
            "Skipped recovered_from_paused side effects for {} (stale event)",
            subscription_id
        );
        return Ok(());
    }

    // Send notification with resume method
    if let Ok(Some(email)) = state.database.get_user_email(clerk_id).await {
        let resume_type = if is_manual_resume { "manually resumed" } else { "automatically resumed" };
        crate::services::email::send_email_mock(
            &email,
            "Subscription Resumed",
            &format!(
                "Your subscription has been {}. \
                You now have full access to premium features. Subscription: {}",
                resume_type,
                subscription_id
            ),
        );
    }

    info!(
        "Successfully recovered subscription {} from paused (manual: {})",
        subscription_id, is_manual_resume
    );

    Ok(())
}

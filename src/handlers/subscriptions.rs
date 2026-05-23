use crate::config::API_PAGINATION_LIMIT;
use crate::config::MAX_PAGINATION_LIMIT;
use crate::application::subscription_status::{self, SubscriptionStatusInput, SubscriptionStatusSnapshot};
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::ports::SubscriptionReadRepository;
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use base64::Engine;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListSubscriptionsQuery {
    pub external_user_id: String,
    pub limit: Option<i64>,
    pub after: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SubscriptionCursor {
    created_at: DateTime<Utc>,
    id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionDetail {
    pub id: String,
    pub subscription_id: String,
    pub provider: String,
    pub status: String,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
    pub payment_failure_notification: bool,
    pub payment_state: Option<i32>,
    pub cancel_reason: Option<i32>,
    pub provider_customer_id: Option<String>,
    pub cancellation_initiated_at: Option<String>,
    pub revocation_reason: Option<String>,
    pub revoked_at: Option<String>,
    pub google_requires_price_step_up_consent: Option<bool>,
    pub google_price_step_up_consent_deadline: Option<String>,
    pub google_new_price_cents: Option<i32>,
    pub google_pending_price_change_new_price_cents: Option<i64>,
    pub google_pending_price_change_currency: Option<String>,
    pub google_pending_price_change_mode: Option<String>,
    pub google_pending_price_change_state: Option<String>,
    pub google_pending_price_change_expected_at: Option<String>,
    pub google_pause_scheduled_at: Option<String>,
    pub google_paused_at: Option<String>,
    pub google_deferred_until: Option<String>,
    pub last_event_time: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionDetailFull {
    pub id: String,
    pub subscription_id: String,
    pub provider: String,
    pub status: String,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub cancel_reason: Option<i32>,
    pub provider_customer_id: Option<String>,
    pub cancellation_initiated_at: Option<String>,
    pub revocation_reason: Option<String>,
    pub revoked_at: Option<String>,
    pub google_requires_price_step_up_consent: Option<bool>,
    pub google_price_step_up_consent_deadline: Option<String>,
    pub google_new_price_cents: Option<i32>,
    pub google_pending_price_change_new_price_cents: Option<i64>,
    pub google_pending_price_change_currency: Option<String>,
    pub google_pending_price_change_mode: Option<String>,
    pub google_pending_price_change_state: Option<String>,
    pub google_pending_price_change_expected_at: Option<String>,
    pub google_pause_scheduled_at: Option<String>,
    pub google_paused_at: Option<String>,
    pub google_deferred_until: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaginationMeta {
    pub has_more: bool,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListSubscriptionsResponse {
    pub subscriptions: Vec<SubscriptionDetail>,
    pub pagination: PaginationMeta,
}

pub async fn list_subscriptions(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Query(query): Query<ListSubscriptionsQuery>,
) -> Result<(StatusCode, Json<ListSubscriptionsResponse>), BridgeError> {
    let database = state.database();
    if query.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }

    let limit = query.limit.unwrap_or(API_PAGINATION_LIMIT).min(MAX_PAGINATION_LIMIT);

    let cursor = decode_cursor(query.after.as_deref())?;

    let subs = database.as_ref().get_user_subscriptions_keyset(
        auth.app_id,
        &query.external_user_id,
        limit + 1, // Fetch one extra to check if there are more
        cursor.as_ref().map(|c| c.created_at),
        cursor.as_ref().map(|c| c.id),
    )
    .await?;

    let has_more = subs.len() > limit as usize;
    let page_subs: Vec<_> = subs.iter().take(limit as usize).collect();
    let subscriptions: Vec<_> = page_subs
        .iter()
        .map(|s| SubscriptionDetail {
            id: s.id.to_string(),
            subscription_id: s.subscription_id.clone(),
            provider: s.provider.clone(),
            status: s.status.clone(),
            current_period_end: s.current_period_end.map(|d| d.to_rfc3339()),
            auto_renewing: s.auto_renewing,
            payment_failure_notification: s.payment_failure_notification,
            payment_state: s.payment_state,
            cancel_reason: s.cancel_reason,
            provider_customer_id: s.provider_customer_id.clone(),
            cancellation_initiated_at: s.cancellation_initiated_at.map(|d| d.to_rfc3339()),
            revocation_reason: s.revocation_reason.clone(),
            revoked_at: s.revoked_at.map(|d| d.to_rfc3339()),
            google_requires_price_step_up_consent: s.google_requires_price_step_up_consent,
            google_price_step_up_consent_deadline: s.google_price_step_up_consent_deadline.map(|d| d.to_rfc3339()),
            google_new_price_cents: s.google_new_price_cents,
            google_pending_price_change_new_price_cents: s.google_pending_price_change_new_price_cents,
            google_pending_price_change_currency: s.google_pending_price_change_currency.clone(),
            google_pending_price_change_mode: s.google_pending_price_change_mode.clone(),
            google_pending_price_change_state: s.google_pending_price_change_state.clone(),
            google_pending_price_change_expected_at: s.google_pending_price_change_expected_at.map(|d| d.to_rfc3339()),
            google_pause_scheduled_at: s.google_pause_scheduled_at.map(|d| d.to_rfc3339()),
            google_paused_at: s.google_paused_at.map(|d| d.to_rfc3339()),
            google_deferred_until: s.google_deferred_until.map(|d| d.to_rfc3339()),
            last_event_time: Some(s.last_event_time),
        })
        .collect();

    let next_cursor = if has_more {
        let last = page_subs.last().expect("has_more implies non-empty page");
        let cursor = SubscriptionCursor { created_at: last.created_at, id: last.id };
        Some(encode_cursor(&cursor)?)
    } else {
        None
    };

    Ok((
        StatusCode::OK,
        Json(ListSubscriptionsResponse {
            subscriptions,
            pagination: PaginationMeta {
                has_more,
                after: next_cursor,
            },
        }),
    ))
}

pub async fn get_subscription_status_snapshot(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(external_user_id): Path<String>,
) -> Result<(StatusCode, Json<SubscriptionStatusSnapshot>), BridgeError> {
    let database = state.database();
    let snapshot = subscription_status::get_subscription_status_snapshot(
        database.as_ref(),
        SubscriptionStatusInput {
            app_id: auth.app_id,
            external_user_id: &external_user_id,
        },
    )
    .await?;

    Ok((StatusCode::OK, Json(snapshot)))
}

fn decode_cursor(after: Option<&str>) -> Result<Option<SubscriptionCursor>, BridgeError> {
    let Some(raw) = after else {
        return Ok(None);
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| BridgeError::ValidationError("Invalid cursor encoding".to_string()))?;
    let cursor: SubscriptionCursor = serde_json::from_slice(&decoded)
        .map_err(|_| BridgeError::ValidationError("Invalid cursor payload".to_string()))?;
    Ok(Some(cursor))
}

fn encode_cursor(cursor: &SubscriptionCursor) -> Result<String, BridgeError> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to encode cursor: {}", e)))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[derive(Debug, Deserialize)]
pub struct GetSubscriptionQuery {
    pub external_user_id: String,
    pub provider: String,
}

pub async fn get_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Query(query): Query<GetSubscriptionQuery>,
) -> Result<(StatusCode, Json<SubscriptionDetailFull>), BridgeError> {
    let database = state.database();
    if query.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }

    if query.provider.is_empty() {
        return Err(BridgeError::ValidationError(
            "provider is required".to_string(),
        ));
    }

    let sub = database.as_ref().get_subscription(
        auth.app_id,
        &query.external_user_id,
        &subscription_id,
        &query.provider,
    )
    .await?;

    let detail = SubscriptionDetailFull {
        id: sub.id.to_string(),
        subscription_id: sub.subscription_id,
        provider: sub.provider,
        status: sub.status,
        current_period_end: sub.current_period_end.map(|d| d.to_rfc3339()),
        auto_renewing: sub.auto_renewing,
        payment_state: sub.payment_state,
        cancel_reason: sub.cancel_reason,
        provider_customer_id: sub.provider_customer_id,
        cancellation_initiated_at: sub.cancellation_initiated_at.map(|d| d.to_rfc3339()),
        revocation_reason: sub.revocation_reason,
        revoked_at: sub.revoked_at.map(|d| d.to_rfc3339()),
        google_requires_price_step_up_consent: sub.google_requires_price_step_up_consent,
        google_price_step_up_consent_deadline: sub.google_price_step_up_consent_deadline.map(|d| d.to_rfc3339()),
        google_new_price_cents: sub.google_new_price_cents,
        google_pending_price_change_new_price_cents: sub.google_pending_price_change_new_price_cents,
        google_pending_price_change_currency: sub.google_pending_price_change_currency,
        google_pending_price_change_mode: sub.google_pending_price_change_mode,
        google_pending_price_change_state: sub.google_pending_price_change_state,
        google_pending_price_change_expected_at: sub.google_pending_price_change_expected_at.map(|d| d.to_rfc3339()),
        google_pause_scheduled_at: sub.google_pause_scheduled_at.map(|d| d.to_rfc3339()),
        google_paused_at: sub.google_paused_at.map(|d| d.to_rfc3339()),
        google_deferred_until: sub.google_deferred_until.map(|d| d.to_rfc3339()),
    };

    Ok((StatusCode::OK, Json(detail)))
}

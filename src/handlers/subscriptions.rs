use crate::db;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use axum::{
    extract::{State, Extension, Query, Path},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use base64::Engine;

#[derive(Debug, Deserialize)]
pub struct ListSubscriptionsQuery {
    pub external_user_id: String,
    pub limit: Option<i64>,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionDetail {
    pub id: String,
    pub subscription_id: String,
    pub provider: String,
    pub status: String,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
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
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Query(query): Query<ListSubscriptionsQuery>,
) -> Result<(StatusCode, Json<ListSubscriptionsResponse>), BridgeError> {
    if query.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }

    let limit = query.limit.unwrap_or(10).min(100);
    
    // Decode cursor to get offset (cursor = base64(offset))
    let offset = if let Some(cursor) = &query.after {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(cursor)
                .unwrap_or_default()
        )
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
    } else {
        0
    };

    let subs = db::subscriptions::get_user_subscriptions(
        &database.pool,
        auth.app_id,
        &query.external_user_id,
        limit + 1, // Fetch one extra to check if there are more
        offset,
    )
    .await?;

    let has_more = subs.len() > limit as usize;
    let subscriptions: Vec<_> = subs
        .into_iter()
        .take(limit as usize)
        .map(|s| SubscriptionDetail {
            id: s.id.to_string(),
            subscription_id: s.subscription_id,
            provider: s.provider,
            status: s.status,
            current_period_end: s.current_period_end.map(|d| d.to_rfc3339()),
            auto_renewing: s.auto_renewing,
        })
        .collect();

    let next_cursor = if has_more {
        let next_offset = offset + limit;
        Some(base64::engine::general_purpose::STANDARD.encode(next_offset.to_string()))
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

#[derive(Debug, Deserialize)]
pub struct GetSubscriptionQuery {
    pub external_user_id: String,
    pub provider: String,
}

pub async fn get_subscription(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Query(query): Query<GetSubscriptionQuery>,
) -> Result<(StatusCode, Json<SubscriptionDetail>), BridgeError> {
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

    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &query.external_user_id,
        &subscription_id,
        &query.provider,
    )
    .await?;

    let detail = SubscriptionDetail {
        id: sub.id.to_string(),
        subscription_id: sub.subscription_id,
        provider: sub.provider,
        status: sub.status,
        current_period_end: sub.current_period_end.map(|d| d.to_rfc3339()),
        auto_renewing: sub.auto_renewing,
    };

    Ok((StatusCode::OK, Json(detail)))
}

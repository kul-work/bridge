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

#[derive(Debug, Deserialize)]
pub struct ListSubscriptionsQuery {
    pub external_user_id: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
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
pub struct ListSubscriptionsResponse {
    pub subscriptions: Vec<SubscriptionDetail>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
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
    let offset = query.offset.unwrap_or(0).max(0);

    // Get total count
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pay.subscriptions WHERE app_id = $1 AND external_user_id = $2",
    )
    .bind(auth.app_id)
    .bind(&query.external_user_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let subs = db::subscriptions::get_user_subscriptions(
        &database.pool,
        auth.app_id,
        &query.external_user_id,
        limit,
        offset,
    )
    .await?;

    let subscriptions = subs
        .into_iter()
        .map(|s| SubscriptionDetail {
            id: s.id.to_string(),
            subscription_id: s.subscription_id,
            provider: s.provider,
            status: s.status,
            current_period_end: s.current_period_end.map(|d| d.to_rfc3339()),
            auto_renewing: s.auto_renewing,
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(ListSubscriptionsResponse {
            subscriptions,
            total,
            limit,
            offset,
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

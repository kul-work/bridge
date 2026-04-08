use crate::application;
use crate::application::subscription_actions_types::{
    BillingPortalResponse, CancelSubscriptionRequest, CancelSubscriptionResponse,
    PriceStepUpAcceptResponse, PriceStepUpDeclineResponse, PriceStepUpRequest,
    ResumeSubscriptionResponse, SubscriptionActionQuery, SubscriptionActionResponse,
};
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};

pub async fn cancel_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Query(query): Query<SubscriptionActionQuery>,
    request: Option<Json<CancelSubscriptionRequest>>,
) -> Result<(StatusCode, Json<CancelSubscriptionResponse>), BridgeError> {
    let response = application::subscription_actions::cancel_subscription(
        state.app_webhook_repo.as_ref(),
        state.subscription_read_repo.as_ref(),
        state.subscription_write_repo.as_ref(),
        auth.app_id,
        &subscription_id,
        query,
        request.map(|Json(value)| value),
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

pub async fn resume_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Query(query): Query<SubscriptionActionQuery>,
) -> Result<(StatusCode, Json<ResumeSubscriptionResponse>), BridgeError> {
    let response = application::subscription_actions::resume_subscription(
        state.app_webhook_repo.as_ref(),
        state.subscription_read_repo.as_ref(),
        state.subscription_write_repo.as_ref(),
        auth.app_id,
        &subscription_id,
        query,
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

#[derive(Debug, serde::Deserialize)]
pub struct AcknowledgeRequest {
    pub external_user_id: String,
}

pub async fn acknowledge_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<AcknowledgeRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let response = application::subscription_actions::acknowledge_subscription(
        state.app_webhook_repo.as_ref(),
        state.subscription_write_repo.as_ref(),
        auth.app_id,
        &subscription_id,
        &request.external_user_id,
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

pub async fn create_billing_portal(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Query(query): Query<SubscriptionActionQuery>,
) -> Result<(StatusCode, Json<BillingPortalResponse>), BridgeError> {
    let response = application::subscription_actions::create_billing_portal(
        state.app_webhook_repo.as_ref(),
        state.subscription_read_repo.as_ref(),
        auth.app_id,
        &subscription_id,
        query,
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

pub async fn accept_price_step_up(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<PriceStepUpRequest>,
) -> Result<(StatusCode, Json<PriceStepUpAcceptResponse>), BridgeError> {
    let response = application::subscription_actions::accept_price_step_up(
        state.app_webhook_repo.as_ref(),
        state.subscription_write_repo.as_ref(),
        auth.app_id,
        &subscription_id,
        request,
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

pub async fn decline_price_step_up(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<PriceStepUpRequest>,
) -> Result<(StatusCode, Json<PriceStepUpDeclineResponse>), BridgeError> {
    let response = application::subscription_actions::decline_price_step_up(
        state.app_webhook_repo.as_ref(),
        state.subscription_write_repo.as_ref(),
        auth.app_id,
        &subscription_id,
        request,
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

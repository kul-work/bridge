use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::application;
use crate::application::checkout_types::{CheckoutRequest, CheckoutResponse};
use crate::state::AppState;
use axum::{
    extract::{State, Extension},
    http::StatusCode,
    Json,
};

pub async fn create_checkout(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<CheckoutRequest>,
) -> Result<(StatusCode, Json<CheckoutResponse>), BridgeError> {
    let repo = state.checkout_repo();
    let response = application::checkout::create_checkout(
        repo,
        auth.app_id,
        payload,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(response)))
}

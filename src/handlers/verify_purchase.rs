use crate::application;
use crate::application::verify_purchase_types::{ProductType, VerifyPurchaseRequest, VerifyPurchaseResponse};
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::state::AppState;
use axum::{
    extract::{Extension, State},
    http::StatusCode,
    Json,
};

pub async fn verify_purchase(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<VerifyPurchaseRequest>,
) -> Result<(StatusCode, Json<VerifyPurchaseResponse>), BridgeError> {
    let repo = state.verify_purchase_repo();
    let response = application::verify_purchase::verify_purchase(
        repo,
        auth.app_id,
        payload.clone(),
    )
    .await?;
    let product_type = ProductType::parse(&payload.product_type)?;
    let status_code = if payload.provider == "google_play"
        && !product_type.is_subscription()
        && response.status == "pending"
    {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };

    Ok((status_code, Json(response)))
}

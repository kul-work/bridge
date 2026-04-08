use crate::application;
use crate::application::verify_purchase_types::{VerifyPurchaseRequest, VerifyPurchaseResponse};
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
    let database = state.database();
    let response = application::verify_purchase::verify_purchase(
        database.as_ref(),
        auth.app_id,
        payload,
    )
    .await?;

    Ok((StatusCode::OK, Json(response)))
}

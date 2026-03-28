use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use chrono::Utc;

use crate::db::Database;
use crate::db::apps::App;
use crate::error::BridgeError;

#[derive(Deserialize)]
pub struct AnonymizeRequest {
    pub reason: Option<String>,
}

pub async fn anonymize(
    State(database): State<Arc<Database>>,
    Extension(app): Extension<App>,
    Path(external_user_id): Path<String>,
    Json(request): Json<AnonymizeRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let (subscriptions_cancelled, payments_anonymized, new_anonymous_id) = 
        crate::db::users::anonymize_user(
            &database.pool,
            app.id,
            &external_user_id,
            request.reason.as_deref(),
        )
        .await?;

    if subscriptions_cancelled == 0 && payments_anonymized == 0 {
        return Err(BridgeError::ValidationError("User not found".to_string()));
    }

    Ok(Json(json!({
        "anonymized": true,
        "subscriptions_cancelled": subscriptions_cancelled,
        "payments_anonymized": payments_anonymized,
        "new_anonymous_id": new_anonymous_id
    })))
}

pub async fn data_export(
    State(database): State<Arc<Database>>,
    Extension(app): Extension<App>,
    Path(external_user_id): Path<String>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let subscriptions = crate::db::subscriptions::get_user_subscriptions(
        &database.pool,
        app.id,
        &external_user_id,
        100,
        0,
    )
    .await?;
    
    let payments = crate::db::payments::get_user_payments(
        &database.pool,
        app.id,
        &external_user_id,
        100,
        0,
    )
    .await.unwrap_or_default();

    Ok(Json(json!({
        "external_user_id": external_user_id,
        "export_date": Utc::now(),
        "subscriptions": subscriptions,
        "payments": payments
    })))
}

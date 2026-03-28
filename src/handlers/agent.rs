use axum::{
    extract::{Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::Database;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;

#[derive(Deserialize)]
pub struct AgentBalanceQuery {
    pub external_user_id: String,
}

#[derive(Deserialize)]
pub struct AgentTokenRequest {
    pub external_user_id: String,
    pub endpoint: String,
    pub amount_cents: i32,
    pub nonce: String,
}

#[derive(Deserialize)]
pub struct AgentChargeRequest {
    pub external_user_id: String,
    pub token_id: Uuid,
}

#[derive(Deserialize)]
pub struct AgentTopUpRequest {
    pub external_user_id: String,
    pub amount_cents: i32,
    pub charge_id: Option<String>,
}

pub async fn balance(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Query(query): Query<AgentBalanceQuery>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let credit = crate::db::agent::get_agent_credit(&database.pool, auth.app_id, &query.external_user_id).await?;
    
    Ok(Json(json!({
        "external_user_id": query.external_user_id,
        "balance_cents": credit.as_ref().map(|c| c.balance_cents).unwrap_or(0),
        "lifetime_spent_cents": credit.as_ref().map(|c| c.lifetime_spent_cents).unwrap_or(0)
    })))
}

pub async fn token(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentTokenRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let token = crate::db::agent::insert_agent_token(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &request.endpoint,
        request.amount_cents,
        &request.nonce,
    )
    .await?;

    Ok(Json(json!({
        "token_id": token.id,
        "amount_cents": token.amount_cents,
        "expires_at": token.expires_at
    })))
}

pub async fn charge(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentChargeRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let new_balance = crate::db::agent::charge_agent(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        request.token_id,
    )
    .await?;

    Ok(Json(json!({
        "charged": true,
        "amount_cents": 0, // In practice, would return token amount.
        "new_balance_cents": new_balance
    })))
}

pub async fn topup(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentTopUpRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let credit = crate::db::agent::upsert_agent_credit(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        request.amount_cents,
        0,
    )
    .await?;

    let mut tx = database.pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    crate::db::agent::record_agent_transaction(
        &mut tx,
        auth.app_id,
        &request.external_user_id,
        "topup",
        request.amount_cents,
        request.charge_id.as_deref(),
    ).await?;
    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(Json(json!({
        "credited": true,
        "new_balance_cents": credit.balance_cents
    })))
}

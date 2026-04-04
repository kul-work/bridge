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
    pub endpoint: String,
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
    if request.amount_cents <= 0 {
        return Err(BridgeError::ValidationError("amount_cents must be positive".to_string()));
    }
    if request.endpoint.trim().is_empty() {
        return Err(BridgeError::ValidationError("endpoint is required".to_string()));
    }

    // §40: Ensure agent_credits row exists (tokens must be backed by credits)
    let credit = crate::db::agent::get_agent_credit(&database.pool, auth.app_id, &request.external_user_id).await?;
    if credit.is_none() {
        return Err(BridgeError::ValidationError("User has no agent credits account".to_string()));
    }

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
    let (new_balance, amount_charged) = crate::db::agent::charge_agent(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        request.token_id,
        &request.endpoint,
    )
    .await?;

    Ok(Json(json!({
        "charged": true,
        "amount_cents": amount_charged,
        "new_balance_cents": new_balance
    })))
}

pub async fn topup(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentTopUpRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let mut tx = database.pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    
    // Upsert credit within transaction
    let credit = sqlx::query_as::<_, crate::db::agent::AgentCredit>(
        r#"
        INSERT INTO pay.agent_credits (app_id, external_user_id, balance_cents, lifetime_spent_cents, updated_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (app_id, external_user_id)
        DO UPDATE SET
            balance_cents = pay.agent_credits.balance_cents + EXCLUDED.balance_cents,
            lifetime_spent_cents = pay.agent_credits.lifetime_spent_cents + EXCLUDED.lifetime_spent_cents,
            updated_at = NOW()
        RETURNING *
        "#
    )
    .bind(auth.app_id)
    .bind(&request.external_user_id)
    .bind(request.amount_cents)
    .bind(0)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

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

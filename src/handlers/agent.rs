use axum::{
    extract::{Query, State},
    Extension, Json,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::OnceLock;
use uuid::Uuid;

use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::state::AppState;

const AGENT_TOKEN_EXPIRES_IN_SECONDS: i64 = 600;
const SUPPORTED_AGENT_ENDPOINTS: [&str; 2] = ["story", "joke"];
static AGENT_EMAIL_REGEX: OnceLock<Regex> = OnceLock::new();

#[derive(Deserialize)]
pub struct AgentBalanceQuery {
    pub external_user_id: String,
}

#[derive(Deserialize)]
pub struct AgentTokenRequest {
    pub external_user_id: String,
    pub endpoint: String,
    pub amount_cents: i32,
}

#[derive(Serialize)]
pub struct AgentTokenResponse {
    pub token: String,
    pub expires_in_seconds: i64,
}

#[derive(Deserialize)]
pub struct AgentChargeRequest {
    pub external_user_id: String,
    pub token: String,
    pub endpoint: String,
}

#[derive(Serialize)]
pub struct AgentChargeResponse {
    pub charged: bool,
    pub amount_cents: i32,
}

#[derive(Deserialize)]
pub struct AgentTopUpRequest {
    pub external_user_id: String,
    pub amount_cents: i32,
    pub charge_id: Option<String>,
}

pub async fn balance(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Query(query): Query<AgentBalanceQuery>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let credit = state
        .agent_repo
        .get_agent_credit(auth.app_id, &query.external_user_id)
        .await?;

    Ok(Json(json!({
        "external_user_id": query.external_user_id,
        "balance_cents": credit.as_ref().map(|c| c.balance_cents).unwrap_or(0),
        "lifetime_spent_cents": credit.as_ref().map(|c| c.lifetime_spent_cents).unwrap_or(0)
    })))
}

pub async fn token(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentTokenRequest>,
) -> Result<Json<AgentTokenResponse>, BridgeError> {
    let external_user_id = normalize_agent_required_field(&request.external_user_id, "external_user_id")?;
    validate_agent_email(&external_user_id)?;

    let endpoint = normalize_agent_endpoint(&request.endpoint)?;

    if request.amount_cents <= 0 {
        return Err(BridgeError::ValidationError("amount_cents must be positive".to_string()));
    }

    // §40: Ensure agent_credits row exists with a zero balance if missing.
    let credit = match state
        .agent_repo
        .get_agent_credit(auth.app_id, &external_user_id)
        .await?
    {
        Some(credit) => credit,
        None => state
            .agent_repo
            .upsert_agent_credit(auth.app_id, &external_user_id, 0, 0)
            .await?,
    };

    if credit.balance_cents < 0 {
        return Err(BridgeError::ValidationError(
            "Agent credit balance cannot be negative".to_string(),
        ));
    }

    let nonce = Uuid::new_v4().to_string();
    let token = state
        .agent_repo
        .insert_agent_token(
            auth.app_id,
            &external_user_id,
            &endpoint,
            request.amount_cents,
            &nonce,
        )
        .await?;

    Ok(Json(AgentTokenResponse {
        token: token.id.to_string(),
        expires_in_seconds: AGENT_TOKEN_EXPIRES_IN_SECONDS,
    }))
}

pub async fn charge(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentChargeRequest>,
) -> Result<Json<AgentChargeResponse>, BridgeError> {
    let external_user_id = normalize_agent_required_field(&request.external_user_id, "external_user_id")?;
    validate_agent_email(&external_user_id)?;

    let endpoint = normalize_agent_endpoint(&request.endpoint)?;
    let token_id = Uuid::parse_str(request.token.trim()).map_err(|_| {
        BridgeError::ValidationError("token must be a valid UUID".to_string())
    })?;

    let (_new_balance, amount_charged) = state
        .agent_repo
        .charge_agent(auth.app_id, &external_user_id, token_id, &endpoint)
        .await?;

    Ok(Json(AgentChargeResponse {
        charged: true,
        amount_cents: amount_charged,
    }))
}

pub async fn topup(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<AgentTopUpRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let credit = state
        .agent_repo
        .topup_agent(
            auth.app_id,
            &request.external_user_id,
            request.amount_cents,
            request.charge_id.as_deref(),
        )
        .await?;

    Ok(Json(json!({
        "credited": true,
        "new_balance_cents": credit.balance_cents
    })))
}

fn normalize_agent_required_field(value: &str, field_name: &str) -> Result<String, BridgeError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(BridgeError::ValidationError(format!("{field_name} is required")));
    }

    Ok(normalized.to_string())
}

fn validate_agent_email(email: &str) -> Result<(), BridgeError> {
    let email_regex = AGENT_EMAIL_REGEX.get_or_init(|| {
        Regex::new(r"(?i)^[^@\s]+@[^@\s]+\.[^@\s]+$")
            .expect("agent email validation regex must compile")
    });

    if email_regex.is_match(email) {
        Ok(())
    } else {
        Err(BridgeError::ValidationError(
            "external_user_id must be a valid email address".to_string(),
        ))
    }
}

fn normalize_agent_endpoint(endpoint: &str) -> Result<String, BridgeError> {
    let normalized = normalize_agent_required_field(endpoint, "endpoint")?.to_ascii_lowercase();
    if SUPPORTED_AGENT_ENDPOINTS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(BridgeError::ValidationError(format!(
            "endpoint must be one of: {}",
            SUPPORTED_AGENT_ENDPOINTS.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_agent_endpoint, validate_agent_email};

    #[test]
    fn validate_agent_email_accepts_basic_email_addresses() {
        assert!(validate_agent_email("agent@example.com").is_ok());
        assert!(validate_agent_email("Agent.User+test@example.co.uk").is_ok());
    }

    #[test]
    fn validate_agent_email_rejects_invalid_values() {
        assert!(validate_agent_email("not-an-email").is_err());
        assert!(validate_agent_email("agent@").is_err());
        assert!(validate_agent_email("agent example.com").is_err());
    }

    #[test]
    fn normalize_agent_endpoint_accepts_supported_endpoints() {
        assert_eq!(normalize_agent_endpoint("story").unwrap(), "story");
        assert_eq!(normalize_agent_endpoint(" JOKE ").unwrap(), "joke");
    }

    #[test]
    fn normalize_agent_endpoint_rejects_unsupported_endpoints() {
        assert!(normalize_agent_endpoint("billing").is_err());
    }
}

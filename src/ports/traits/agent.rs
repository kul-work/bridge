use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::agent::{AgentCredit, AgentTransaction},
    error::BridgeError,
};

#[async_trait]
pub trait AgentReadRepository: Send + Sync {
    async fn get_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Option<AgentCredit>, BridgeError>;

    async fn list_agent_transactions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Vec<AgentTransaction>, BridgeError>;
}

#[async_trait]
pub trait AgentRepository: AgentReadRepository + Send + Sync {
    async fn upsert_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        balance_delta: i32,
        spent_delta: i32,
    ) -> Result<AgentCredit, BridgeError>;

    async fn insert_agent_token(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        endpoint: &str,
        amount_cents: i32,
        nonce: &str,
    ) -> Result<crate::db::agent::AgentPaymentToken, BridgeError>;

    async fn charge_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        token_id: Uuid,
        endpoint: &str,
    ) -> Result<(i32, i32), BridgeError>;

    async fn topup_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        amount_cents: i32,
        charge_id: Option<&str>,
    ) -> Result<AgentCredit, BridgeError>;
}

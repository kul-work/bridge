use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self, agent::{AgentCredit, AgentTransaction}},
    error::BridgeError,
    ports::traits::{AgentReadRepository, AgentRepository},
};

#[async_trait]
impl AgentReadRepository for db::Database {
    async fn get_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Option<AgentCredit>, BridgeError> {
        db::agent::get_agent_credit(self.pool(), app_id, external_user_id).await
    }

    async fn list_agent_transactions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Vec<AgentTransaction>, BridgeError> {
        db::agent::list_agent_transactions(self.pool(), app_id, external_user_id).await
    }
}

#[async_trait]
impl AgentRepository for db::Database {
    async fn upsert_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        balance_delta: i32,
        spent_delta: i32,
    ) -> Result<AgentCredit, BridgeError> {
        db::agent::upsert_agent_credit(
            self.pool(),
            app_id,
            external_user_id,
            balance_delta,
            spent_delta,
        )
        .await
    }

    async fn insert_agent_token(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        endpoint: &str,
        amount_cents: i32,
        nonce: &str,
    ) -> Result<crate::db::agent::AgentPaymentToken, BridgeError> {
        db::agent::insert_agent_token(
            self.pool(),
            app_id,
            external_user_id,
            endpoint,
            amount_cents,
            nonce,
        )
        .await
    }

    async fn charge_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        token_id: Uuid,
        endpoint: &str,
    ) -> Result<(i32, i32), BridgeError> {
        db::agent::charge_agent(self.pool(), app_id, external_user_id, token_id, endpoint).await
    }

    async fn topup_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        amount_cents: i32,
        charge_id: Option<&str>,
    ) -> Result<AgentCredit, BridgeError> {
        db::agent::topup_agent(self.pool(), app_id, external_user_id, amount_cents, charge_id).await
    }
}

use std::sync::Arc;

use axum::extract::FromRef;

use crate::{
    db::Database,
    ports::{
        AdminRepository, AgentReadRepository, AgentRepository, ApiKeyRepository,
        AppProviderRepository, CheckoutRepository, PaymentReadRepository,
        SubscriptionReadRepository, SubscriptionWriteRepository, UserRepository,
        WebhookIngressRepository, WebhookReadRepository,
    },
};

#[derive(Clone)]
pub struct AppState {
    database: Arc<Database>,
    pub api_key_repo: Arc<dyn ApiKeyRepository>,
    pub admin_repo: Arc<dyn AdminRepository>,
    pub app_provider_repo: Arc<dyn AppProviderRepository>,
    pub checkout_repo: Arc<dyn CheckoutRepository>,
    pub webhook_ingress_repo: Arc<dyn WebhookIngressRepository>,
    pub subscription_read_repo: Arc<dyn SubscriptionReadRepository>,
    pub subscription_write_repo: Arc<dyn SubscriptionWriteRepository>,
    pub payment_read_repo: Arc<dyn PaymentReadRepository>,
    pub user_repo: Arc<dyn UserRepository>,
    pub agent_read_repo: Arc<dyn AgentReadRepository>,
    pub agent_repo: Arc<dyn AgentRepository>,
    pub webhook_read_repo: Arc<dyn WebhookReadRepository>,
}

impl AppState {
    pub fn new(database: Arc<Database>) -> Self {
        let api_key_repo: Arc<dyn ApiKeyRepository> = database.clone();
        let admin_repo: Arc<dyn AdminRepository> = database.clone();
        let app_provider_repo: Arc<dyn AppProviderRepository> = database.clone();
        let checkout_repo: Arc<dyn CheckoutRepository> = database.clone();
        let webhook_ingress_repo: Arc<dyn WebhookIngressRepository> = database.clone();
        let subscription_read_repo: Arc<dyn SubscriptionReadRepository> = database.clone();
        let subscription_write_repo: Arc<dyn SubscriptionWriteRepository> = database.clone();
        let payment_read_repo: Arc<dyn PaymentReadRepository> = database.clone();
        let user_repo: Arc<dyn UserRepository> = database.clone();
        let agent_read_repo: Arc<dyn AgentReadRepository> = database.clone();
        let agent_repo: Arc<dyn AgentRepository> = database.clone();
        let webhook_read_repo: Arc<dyn WebhookReadRepository> = database.clone();

        Self {
            database,
            api_key_repo,
            admin_repo,
            app_provider_repo,
            checkout_repo,
            webhook_ingress_repo,
            subscription_read_repo,
            subscription_write_repo,
            payment_read_repo,
            user_repo,
            agent_read_repo,
            agent_repo,
            webhook_read_repo,
        }
    }
}

impl FromRef<AppState> for Arc<Database> {
    fn from_ref(state: &AppState) -> Self {
        state.database.clone()
    }
}

impl AppState {
    pub fn database(&self) -> Arc<Database> {
        self.database.clone()
    }
}

use std::sync::Arc;

use axum::extract::FromRef;

use crate::{
    db::Database,
    ports::{SubscriptionActionsHandlerRepository, VerifyPurchaseHandlerRepository},
};

#[derive(Clone)]
pub struct AppState {
    database: Arc<Database>,
}

impl AppState {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
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

    pub(crate) fn verify_purchase_repo(&self) -> &(dyn VerifyPurchaseHandlerRepository + '_) {
        self.database.as_ref()
    }

    pub(crate) fn subscription_actions_repo(&self) -> &(dyn SubscriptionActionsHandlerRepository + '_) {
        self.database.as_ref()
    }
}

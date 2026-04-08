use std::sync::Arc;

use axum::extract::FromRef;

use crate::{
    db::Database,
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
}

pub mod ingress;
pub mod processor;
pub mod forwarding;

use axum::{
    routing::post,
    Router,
};
use std::sync::Arc;

use crate::db::Database;

/// Build webhook routes
pub fn webhook_routes(database: Arc<Database>) -> Router<Arc<Database>> {
    Router::new()
        .route("/:token/google_play", post(ingress::handle_google_play))
        .route("/:token/creem", post(ingress::handle_creem))
        .route("/:token/lemonsqueezy", post(ingress::handle_lemonsqueezy))
        .route("/:token/coinbase", post(ingress::handle_coinbase))
        .with_state(database)
}

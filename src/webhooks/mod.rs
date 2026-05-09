pub mod ingress;
pub mod processor;
pub mod forwarding;
pub mod scheduler;

use axum::{
    routing::post,
    Router,
};

use crate::state::AppState;

/// Build webhook routes
pub fn webhook_routes() -> Router<AppState> {
    Router::new()
        .route("/:token/google_play", post(ingress::handle_google_play))
        .route("/:token/creem", post(ingress::handle_creem))
}

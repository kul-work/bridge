// Only the modules reached from `main.rs` (bootstrap) or from integration
// tests (Phase 1+) are `pub`. The rest stay crate-private so `pub fn`s inside
// them that expose `pub(crate)` types don't trip the `private_interfaces` lint.
mod application;
pub mod config;
pub mod db;
mod error;
pub mod handlers;
pub mod middleware;
mod ports;
pub mod services;
pub mod state;
mod utils;
pub mod webhooks;

use axum::{http::StatusCode, response::Redirect, routing::get, Router};

use crate::{
    config::{is_production_environment, Config},
    handlers::{
        health_check, list_routes, openapi_spec, plain_routes, readiness_check, RouteDescriptor,
    },
    state::AppState,
};

/// Static metadata for every route that should appear in the internal route
/// index / OpenAPI spec. Kept here (next to the router wiring) so it stays in
/// sync with the actual mounts below. The `/routes` and `/routes/openapi`
/// handlers read this list; production skips mounting those handlers entirely.
///
/// `group` is the OpenAPI tag / UI grouping. `auth` is a short label for the
/// JSON index describing how the route is protected. Both are `&'static str`
/// because `RouteDescriptor` stores them as such (see `handlers::routes`).
fn known_routes() -> Vec<RouteDescriptor> {
    vec![
        // --- liveness ---
        rd("GET", "/health", "liveness", "none"),
        rd("GET", "/ready", "liveness", "none"),
        // --- admin (Clerk JWT) ---
        rd("GET", "/admin/apps", "admin", "clerk_jwt"),
        rd("GET", "/admin/alerts", "admin", "clerk_jwt"),
        rd("PATCH", "/admin/apps/:app_id/notes", "admin", "clerk_jwt"),
        rd("GET", "/admin/apps/:app_id/webhooks", "admin", "clerk_jwt"),
        rd("GET", "/admin/webhooks/:webhook_id/payload", "admin", "clerk_jwt"),
        rd("POST", "/admin/webhooks/:webhook_id/retry", "admin", "clerk_jwt"),
        rd("POST", "/admin/trigger-jobs", "admin", "clerk_jwt"),
        // --- api_key (Bearer API key) ---
        rd("POST", "/api/v1/app/verify", "api_key", "api_key"),
        rd("POST", "/api/v1/payment/checkout", "api_key", "api_key"),
        rd("POST", "/api/v1/verify-purchase", "api_key", "api_key"),
        rd("GET", "/api/v1/subscriptions", "api_key", "api_key"),
        rd("GET", "/api/v1/subscriptions/:subscription_id", "api_key", "api_key"),
        rd("POST", "/api/v1/subscriptions/:subscription_id/cancel", "api_key", "api_key"),
        rd("POST", "/api/v1/subscriptions/:subscription_id/resume", "api_key", "api_key"),
        rd("POST", "/api/v1/subscriptions/:subscription_id/acknowledge", "api_key", "api_key"),
        rd("POST", "/api/v1/subscriptions/:subscription_id/portal", "api_key", "api_key"),
        rd("POST", "/api/v1/subscriptions/:subscription_id/price-step-up/accept", "api_key", "api_key"),
        rd("POST", "/api/v1/subscriptions/:subscription_id/price-step-up/decline", "api_key", "api_key"),
        rd("GET", "/api/v1/payments", "api_key", "api_key"),
        rd("POST", "/api/v1/purchase/register", "api_key", "api_key"),
        rd("GET", "/api/v1/users/:external_user_id/subscription-status", "api_key", "api_key"),
        rd("POST", "/api/v1/users/:external_user_id/anonymize", "api_key", "api_key"),
        rd("GET", "/api/v1/users/:external_user_id/data-export", "api_key", "api_key"),
        // --- webhook (token in path) ---
        rd("POST", "/webhooks/:token/google_play", "webhook", "webhook_token"),
        rd("POST", "/webhooks/:token/creem", "webhook", "webhook_token"),
        // --- test (loopback only; mounted only when mock_external_apis) ---
        rd("POST", "/internal/test/log-marker", "test", "loopback"),
    ]
}

/// Small constructor to keep `known_routes()` readable. `group` and `auth`
/// must be string literals because `RouteDescriptor` stores `&'static str`.
fn rd(method: &str, path: &str, group: &'static str, auth: &'static str) -> RouteDescriptor {
    RouteDescriptor {
        method: method.to_string(),
        path: path.to_string(),
        group,
        auth,
    }
}

/// Build the full Axum router. Extracted from `main.rs` so integration tests
/// can construct the app in-process without spawning a server. Bootstrap
/// (tracing, config load, DB pool, background workers) stays in `main.rs`.
///
/// `config` is read here for development-only mounts such as `/internal/test`
/// and the opt-in Swagger route index.
pub fn build_app(config: &Config, app_state: AppState) -> Router {
    // Build protected routes with API key middleware
    let mut protected_routes = Router::new()
        .route(
            "/app/verify",
            axum::routing::post(handlers::api_key::verify_expected_app),
        )
        .route(
            "/payment/checkout",
            axum::routing::post(handlers::checkout::create_checkout),
        )
        .route(
            "/verify-purchase",
            axum::routing::post(handlers::verify_purchase::verify_purchase),
        )
        .route(
            "/subscriptions",
            axum::routing::get(handlers::subscriptions::list_subscriptions),
        )
        .route(
            "/subscriptions/:subscription_id",
            axum::routing::get(handlers::subscriptions::get_subscription),
        )
        .route(
            "/subscriptions/:subscription_id/cancel",
            axum::routing::post(handlers::subscriptions_actions::cancel_subscription),
        )
        .route(
            "/subscriptions/:subscription_id/resume",
            axum::routing::post(handlers::subscriptions_actions::resume_subscription),
        )
        .route(
            "/subscriptions/:subscription_id/acknowledge",
            axum::routing::post(handlers::subscriptions_actions::acknowledge_subscription),
        )
        .route(
            "/subscriptions/:subscription_id/portal",
            axum::routing::post(handlers::subscriptions_actions::create_billing_portal),
        )
        .route(
            "/subscriptions/:subscription_id/price-step-up/accept",
            axum::routing::post(handlers::subscriptions_actions::accept_price_step_up),
        )
        .route(
            "/subscriptions/:subscription_id/price-step-up/decline",
            axum::routing::post(handlers::subscriptions_actions::decline_price_step_up),
        )
        .route(
            "/payments",
            axum::routing::get(handlers::payments::get_payments),
        )
        .route(
            "/purchase/register",
            axum::routing::post(handlers::payments::register_purchase),
        )
        .route(
            "/users/:external_user_id/subscription-status",
            axum::routing::get(handlers::subscriptions::get_subscription_status_snapshot),
        )
        .route(
            "/users/:external_user_id/anonymize",
            axum::routing::post(handlers::users::anonymize),
        )
        .route(
            "/users/:external_user_id/data-export",
            axum::routing::get(handlers::users::data_export),
        );

    if !config.rate_limit_disabled {
        protected_routes = protected_routes.route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            middleware::rate_limit::api_rate_limit_middleware,
        ));
    }

    protected_routes = protected_routes.route_layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        handlers::api_key::api_key_auth,
    ));

    if !config.rate_limit_disabled {
        protected_routes = protected_routes.route_layer(axum::middleware::from_fn(
            middleware::rate_limit::unauthenticated_ip_rate_limit_middleware,
        ));
    }

    // Admin dashboard page is public (Clerk handles auth client-side).
    // Admin API routes require Clerk JWT middleware.
    let admin_page = Router::new()
        .route("/admin", axum::routing::get(handlers::admin::admin_dashboard))
        .route("/admin/", axum::routing::get(handlers::admin::admin_dashboard))
        .route(
            "/admin/favicon.ico",
            axum::routing::get(|| async { StatusCode::NO_CONTENT }),
        )
        .with_state(app_state.clone());

    let mut admin_api = Router::new()
        .route("/admin/apps", axum::routing::get(handlers::admin::list_apps))
        .route(
            "/admin/alerts",
            axum::routing::get(handlers::admin::alert_dashboard),
        )
        .route(
            "/admin/apps/:app_id/notes",
            axum::routing::patch(handlers::admin::update_app_notes),
        )
        .route(
            "/admin/apps/:app_id/webhooks",
            axum::routing::get(handlers::admin::get_app_webhooks),
        )
        .route(
            "/admin/webhooks/:webhook_id/payload",
            axum::routing::get(handlers::admin::get_webhook_payload),
        )
        .route(
            "/admin/webhooks/:webhook_id/retry",
            axum::routing::post(handlers::admin::retry_webhook),
        )
        .route(
            "/admin/trigger-jobs",
            axum::routing::post(handlers::admin::trigger_jobs),
        );

    if !config.rate_limit_disabled {
        admin_api = admin_api.route_layer(axum::middleware::from_fn(
            middleware::rate_limit::admin_rate_limit_middleware,
        ));
    }

    admin_api = admin_api.route_layer(axum::middleware::from_fn(
        middleware::admin_auth::admin_auth_middleware,
    ));

    if !config.rate_limit_disabled {
        admin_api = admin_api.route_layer(axum::middleware::from_fn(
            middleware::rate_limit::admin_auth_ip_rate_limit_middleware,
        ));
    }

    let admin_api = admin_api.with_state(app_state.clone());

    let admin_routes = admin_page
        .merge(admin_api)
        .layer(axum::middleware::from_fn(handlers::admin::admin_no_store_middleware));

    // Build app
    let mut health_routes = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check));

    if !config.rate_limit_disabled {
        health_routes = health_routes.layer(axum::middleware::from_fn(
            middleware::rate_limit::health_ip_rate_limit_middleware,
        ));
    }

    let mut app = Router::new()
        .merge(health_routes)
        .route("/", axum::routing::get(|| async { Redirect::temporary("/admin") }))
        .route(
            "/favicon.ico",
            axum::routing::get(|| async { StatusCode::NO_CONTENT }),
        )
        .merge(admin_routes)
        .nest("/api/v1", protected_routes)
        .nest("/webhooks", webhooks::webhook_routes())
        .route_layer(axum::middleware::from_fn(
            middleware::observability::capture_matched_path,
        ))
        .layer(axum::middleware::from_fn(
            middleware::observability::request_observability,
        ));

    // Internal route index / OpenAPI spec — available only outside production so
    // security scanners (OWASP ZAP) and developers can discover routes in
    // dev/staging without exposing them publicly. Mirrors the household feature.
    if swagger_routes_enabled(config) {
        app = app
            .route("/routes", get(list_routes))
            .route("/routes/openapi", get(openapi_spec))
            .route("/routes/plain", get(plain_routes));
    }

    if config.mock_external_apis {
        app = app.nest("/internal/test", handlers::test_log::routes());
    }

    app.with_state(app_state)
}

fn swagger_routes_enabled(config: &Config) -> bool {
    config.swagger_enabled && !is_production_environment(&config.environment)
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    use super::swagger_routes_enabled;

    fn test_config() -> Config {
        Config {
            database_url: "postgresql://localhost/bridge".to_string(),
            admin_database_url: None,
            server_addr: "0.0.0.0".to_string(),
            server_port: 3000,
            logging_level: "info".to_string(),
            environment: "development".to_string(),
            mock_external_apis: false,
            swagger_enabled: false,
            enable_background_jobs: true,
            rate_limit_disabled: false,
            bypass_admin_auth: false,
        }
    }

    #[test]
    fn swagger_routes_are_opt_in_outside_production() {
        let mut config = test_config();

        assert!(!swagger_routes_enabled(&config));

        config.swagger_enabled = true;

        assert!(swagger_routes_enabled(&config));
    }

    #[test]
    fn swagger_routes_stay_disabled_for_production_aliases() {
        for environment in ["production", "prod", " Production ", " PROD "] {
            let mut config = test_config();
            config.environment = environment.to_string();
            config.swagger_enabled = true;

            assert!(!swagger_routes_enabled(&config));
        }
    }
}

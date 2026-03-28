mod config;
mod error;
mod db;
mod handlers;
mod services;
mod webhooks;
mod middleware;

use axum::{
    routing::get,
    Router,
    middleware::from_fn_with_state,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tower_http::cors::CorsLayer;
use tracing_subscriber::fmt::time::OffsetTime;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

use config::Config;
use db::Database;
use handlers::health_check;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    let environment = std::env::var("ENVIRONMENT")
        .unwrap_or_else(|_| "development".to_string());
    
    let default_filter = match environment.as_str() {
        "production" | "prod" => "bridge=info,axum=info",
        _ => "bridge=debug,axum=debug",
    };
    
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            default_filter.parse().expect("Valid filter string")
        });

    let timer = OffsetTime::new(
        time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC),
        time::format_description::well_known::Rfc3339,
    );

    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("server")
        .filename_suffix("log")
        .build("logs")
        .expect("failed to create log file appender");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_timer(timer)
        .with_writer(std::io::stdout.and(non_blocking))
        .init();

    // Load config
    let config = Config::from_env()?;

    info!("Starting Bridge v{}", env!("CARGO_PKG_VERSION"));
    info!("Environment: {}", config.environment);

    // Initialize database
    let database = Arc::new(
        Database::new(&config.database_url, config.admin_database_url.as_deref()).await?
    );
    info!("Connected to PostgreSQL");

    // Start background webhook delivery job
    webhooks::scheduler::spawn_webhook_retry_worker(database.clone());
    
    // Start background subscription reconciliation job
    webhooks::scheduler::spawn_reconciliation_worker(database.clone());

    // Build protected routes with API key middleware
    let protected_routes = Router::new()
        .route("/checkout", axum::routing::post(handlers::checkout::create_checkout))
        .route("/payment/checkout", axum::routing::post(handlers::checkout::create_checkout))
        .route("/verify-purchase", axum::routing::post(handlers::verify_purchase::verify_purchase))
        .route("/subscriptions", axum::routing::get(handlers::subscriptions::list_subscriptions))
        .route("/subscriptions/:subscription_id", axum::routing::get(handlers::subscriptions::get_subscription))
        .route("/subscriptions/:subscription_id/cancel", axum::routing::post(handlers::subscriptions_actions::cancel_subscription))
        .route("/subscriptions/:subscription_id/resume", axum::routing::post(handlers::subscriptions_actions::resume_subscription))
        .route("/subscriptions/:subscription_id/acknowledge", axum::routing::post(handlers::subscriptions_actions::acknowledge_subscription))
        .route("/subscriptions/:subscription_id/portal", axum::routing::post(handlers::subscriptions_actions::create_billing_portal))
        .route("/subscriptions/:subscription_id/price-step-up/accept", axum::routing::post(handlers::subscriptions_actions::accept_price_step_up))
        .route("/subscriptions/:subscription_id/price-step-up/decline", axum::routing::post(handlers::subscriptions_actions::decline_price_step_up))
        .route("/payments", axum::routing::get(handlers::payments::get_payments))
        .route("/purchases/register", axum::routing::post(handlers::payments::register_purchase))
        .route("/users/:external_user_id/anonymize", axum::routing::post(handlers::users::anonymize))
        .route("/users/:external_user_id/data-export", axum::routing::get(handlers::users::data_export))
        .route("/agent/balance", axum::routing::get(handlers::agent::balance))
        .route("/agent/token", axum::routing::post(handlers::agent::token))
        .route("/agent/charge", axum::routing::post(handlers::agent::charge))
        .route("/agent/topup", axum::routing::post(handlers::agent::topup))
        .layer(axum::middleware::from_fn_with_state(
            database.clone(),
            handlers::api_key::api_key_auth,
        ));

    // Build admin routes with auth middleware
    let admin_routes = Router::new()
        .route("/", axum::routing::get(handlers::admin::admin_dashboard))
        .route("/apps", axum::routing::get(handlers::admin::list_apps))
        .route("/apps/:app_id/webhooks", axum::routing::get(handlers::admin::get_app_webhooks))
        .route("/webhooks/:webhook_id/retry", axum::routing::post(handlers::admin::retry_webhook))
        .layer(from_fn_with_state(
            database.clone(),
            middleware::admin_auth::admin_auth_middleware,
        ))
        .with_state(database.clone());

    // Build app
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/health", get(health_check))
        .nest("/admin", admin_routes)
        .nest("/api/v1", protected_routes)
        .nest("/webhooks", webhooks::webhook_routes(database.clone()))
        .layer(ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(CorsLayer::permissive())
        )
        .with_state(database);

    // Start server
    let addr = SocketAddr::from((
        config.server_addr.parse::<std::net::IpAddr>()?,
        config.server_port,
    ));

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Bridge server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

mod config;
mod application;
mod error;
mod db;
mod handlers;
mod ports;
mod services;
mod webhooks;
mod middleware;
mod state;

use axum::{
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

use tracing_subscriber::fmt::time::OffsetTime;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

use config::Config;
use db::Database;
use handlers::health_check;
use state::AppState;

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
    let env_lower = config.environment.to_ascii_lowercase();
    let is_production = env_lower == "production" || env_lower == "prod";

    if is_production && config.mock_external_apis {
        anyhow::bail!("MOCK_EXTERNAL_APIS=true is not allowed in production");
    }

    info!("Starting Bridge v{}", env!("CARGO_PKG_VERSION"));
    info!("Environment: {}", config.environment);

    let _email_service = services::email::init_email_service(&config)?;

    // Initialize database
    let database = Arc::new(
        Database::new(&config.database_url, config.admin_database_url.as_deref()).await?
    );
    let app_state = AppState::new(database.clone());
    info!("Connected to PostgreSQL");

    // Start background webhook delivery job
    // Start background workers
    if config.enable_background_jobs {
        webhooks::scheduler::spawn_webhook_retry_worker(database.clone());
        webhooks::scheduler::spawn_reconciliation_worker(database.clone());
        webhooks::scheduler::spawn_price_step_up_expiry_worker(database.clone());
        webhooks::scheduler::spawn_pause_scheduler_worker(database.clone());
        webhooks::scheduler::spawn_webhook_cleanup_worker(database.clone());
    } else {
        info!("Background jobs are disabled (ENABLE_BACKGROUND_JOBS=false)");
    }


    // Build protected routes with API key middleware
    let protected_routes = Router::new()
        .route("/checkout", axum::routing::post(handlers::checkout::create_checkout))
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
        .route("/purchase/register", axum::routing::post(handlers::payments::register_purchase))
        .route("/users/:external_user_id/anonymize", axum::routing::post(handlers::users::anonymize))
        .route("/users/:external_user_id/data-export", axum::routing::get(handlers::users::data_export))
        .route("/agent/balance", axum::routing::get(handlers::agent::balance))
        .route("/agent/token", axum::routing::post(handlers::agent::token))
        .route("/agent/charge", axum::routing::post(handlers::agent::charge))
        .route("/agent/topup", axum::routing::post(handlers::agent::topup))
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            middleware::rate_limit::api_rate_limit_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            handlers::api_key::api_key_auth,
        ))
        .layer(axum::middleware::from_fn(
            middleware::rate_limit::unauthenticated_ip_rate_limit_middleware,
        ));

    // Build admin routes with auth middleware
    let admin_routes = Router::new()
        .route("/", axum::routing::get(handlers::admin::admin_dashboard))
        .route("/apps", axum::routing::get(handlers::admin::list_apps))
        .route("/apps/:app_id/webhooks", axum::routing::get(handlers::admin::get_app_webhooks))
        .route("/webhooks/:webhook_id/retry", axum::routing::post(handlers::admin::retry_webhook))
        .layer(axum::middleware::from_fn(middleware::admin_auth::admin_auth_middleware))
        .with_state(app_state.clone());

    // Build app
    let app = Router::new()
        .route("/health", get(health_check))
        .nest("/admin", admin_routes)
        .nest("/api/v1", protected_routes)
        .nest("/webhooks", webhooks::webhook_routes())
        .layer(ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
        )
        .with_state(app_state);

    // Start server
    let addr = SocketAddr::from((
        config.server_addr.parse::<std::net::IpAddr>()?,
        config.server_port,
    ));

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Bridge server listening on {}", addr);

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

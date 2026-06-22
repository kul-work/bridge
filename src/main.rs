mod config;
mod application;
mod error;
mod db;
mod handlers;
mod ports;
mod services;
mod utils;
mod webhooks;
mod middleware;
mod state;

use axum::{
    http::StatusCode,
    response::Redirect,
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

/// Initialize Google Play service account credentials from environment variables.
///
/// In production-like environments, some hosts provide secret files as env var
/// contents. GOOGLE_PLAY_KEY contains the service account JSON and
/// GOOGLE_SERVICE_ACCOUNT_PATH is the file path existing Google Play clients read.
fn init_google_play_credentials(environment: &str) -> anyhow::Result<()> {
    let env_lower = environment.to_ascii_lowercase();
    if !matches!(env_lower.as_str(), "production" | "staging" | "demo") {
        return Ok(());
    }

    if let Ok(key) = std::env::var("GOOGLE_PLAY_KEY") {
        if let Ok(path) = std::env::var("GOOGLE_SERVICE_ACCOUNT_PATH") {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(&path, key)?;
            info!("Initialized Google Play service account credentials");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    let environment = std::env::var("ENVIRONMENT")
        .unwrap_or_else(|_| "development".to_string());
    
    let default_filter = match environment.as_str() {
        //"production" | "prod" => "bridge=info,axum=info",
        "production" | "prod" => "bridge=info,axum=info,BPT-TRACE=info,BPT-RAW=info",
        _ => "bridge=debug,axum=debug,BPT-TRACE=debug,BPT-RAW=debug",
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
    init_google_play_credentials(&config.environment)?;

    let env_lower = config.environment.to_ascii_lowercase();
    let is_production = env_lower == "production" || env_lower == "prod";

    if is_production && config.mock_external_apis {
        anyhow::bail!("MOCK_EXTERNAL_APIS=true is not allowed in production");
    }

    info!("Starting Bridge v{}", env!("CARGO_PKG_VERSION"));
    info!("Environment: {}", config.environment);

    // Log configuration
    if config.mock_external_apis {
        tracing::info!("⚠️ MOCK_EXTERNAL_APIS is ENABLED - Verification checks disabled");
    }
    let email_provider = std::env::var("EMAIL_PROVIDER").unwrap_or_else(|_| "mock".to_string());
    if email_provider == "mock" {
        tracing::info!("⚠️ EMAIL_PROVIDER is 'mock' - Emails will not be sent");
    }

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
        .route("/purchase/register", axum::routing::post(handlers::payments::register_purchase))
        .route("/users/:external_user_id/subscription-status", axum::routing::get(handlers::subscriptions::get_subscription_status_snapshot))
        .route("/users/:external_user_id/anonymize", axum::routing::post(handlers::users::anonymize))
        .route("/users/:external_user_id/data-export", axum::routing::get(handlers::users::data_export))
        
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

    // Admin dashboard page is public (Clerk handles auth client-side).
    // Admin dashboard page is public (Clerk handles auth client-side).
    // Admin API routes require Clerk JWT middleware.
    let admin_page = Router::new()
        .route("/admin", axum::routing::get(handlers::admin::admin_dashboard))
        .route("/admin/", axum::routing::get(handlers::admin::admin_dashboard))
        .route("/admin/favicon.ico", axum::routing::get(|| async { StatusCode::NO_CONTENT }))
        .with_state(app_state.clone());

    let admin_api = Router::new()
        .route("/admin/apps", axum::routing::get(handlers::admin::list_apps))
        .route("/admin/apps/:app_id/notes", axum::routing::patch(handlers::admin::update_app_notes))
        .route("/admin/apps/:app_id/webhooks", axum::routing::get(handlers::admin::get_app_webhooks))
        .route("/admin/webhooks/:webhook_id/payload", axum::routing::get(handlers::admin::get_webhook_payload))
        .route("/admin/webhooks/:webhook_id/retry", axum::routing::post(handlers::admin::retry_webhook))
        .route("/admin/trigger-jobs", axum::routing::post(handlers::admin::trigger_jobs))
        .layer(axum::middleware::from_fn(middleware::admin_auth::admin_auth_middleware))
        .with_state(app_state.clone());

    let admin_routes = admin_page.merge(admin_api);

    // Build app
    let mut app = Router::new()
        .route("/health", get(health_check))
        .route("/", axum::routing::get(|| async { Redirect::temporary("/admin") }))
        .route("/favicon.ico", axum::routing::get(|| async { StatusCode::NO_CONTENT }))
        .merge(admin_routes)
        .nest("/api/v1", protected_routes)
        .nest("/webhooks", webhooks::webhook_routes())
        .layer(ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
        );

    if config.mock_external_apis {
        app = app.nest("/internal/test", handlers::test_log::routes());
    }

    let app = app.with_state(app_state);

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

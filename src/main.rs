use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use tracing::info;
use tracing_subscriber::fmt::time::OffsetTime;

use bridge::{build_app, config::Config, db::Database, services, state::AppState, webhooks};

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
        "production" | "prod" => "bridge=info,axum=info,BPT-TRACE=info",
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
    config.validate_startup()?;
    init_google_play_credentials(&config.environment)?;

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
        Database::new(&config.database_url, config.admin_database_url.as_deref()).await?,
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

    let app: Router = build_app(&config, app_state);

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

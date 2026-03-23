pub mod apps;
pub mod api_keys;
pub mod subscriptions;
pub mod payments;
pub mod provider_configs;
pub mod webhooks;

use crate::error::BridgeError;
use sqlx::PgPool;
use std::str::FromStr;
use tracing::info;

pub struct Database {
    pub pool: PgPool,
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Database {
            pool: self.pool.clone(),
        }
    }
}

impl Database {
    /// Create a new database connection pool and run migrations
    pub async fn new(database_url: &str) -> Result<Self, BridgeError> {
        let mut opts = sqlx::postgres::PgConnectOptions::from_str(database_url)
            .map_err(|_| BridgeError::ConfigError("Failed to parse database URL".to_string()))?;

        // Only set SSL cert for SSL-required connections (e.g., Supabase, Neon)
        if database_url.contains("sslmode=require") {
            let cert_path = if database_url.contains("supabase.com") {
                "./certs/prod-ca-2021.crt"
            } else if database_url.contains("neon.tech") {
                "./certs/isrgrootx1.pem"
            } else {
                "./certs/prod-ca-2021.crt" // default fallback
            };
            opts = opts.ssl_root_cert(cert_path);
        }

        let pool = PgPool::connect_with(opts)
            .await
            .map_err(|e| BridgeError::DbError(format!("Failed to connect to database: {}", e)))?;

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| BridgeError::DbError(format!("Failed to run migrations: {}", e)))?;

        info!("Migrations completed successfully");

        Ok(Self { pool })
    }
}

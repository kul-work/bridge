use crate::error::BridgeError;
use sqlx::PgPool;
use std::str::FromStr;
use tracing::info;

pub struct Database {
    pool: PgPool,
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Database {
            pool: self.pool.clone(),
        }
    }
}

impl Database {
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a new database connection pool and run migrations
    pub async fn new(
        database_url: &str,
        admin_database_url: Option<&str>,
    ) -> Result<Self, BridgeError> {
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

        // Run migrations with admin credentials when available, so runtime app role
        // can stay least-privilege.
        if let Some(admin_url) = admin_database_url {
            let admin_opts = sqlx::postgres::PgConnectOptions::from_str(admin_url)
                .map_err(|_| {
                    BridgeError::ConfigError("Failed to parse ADMIN_DATABASE_URL".to_string())
                })?;
            let admin_pool = PgPool::connect_with(admin_opts).await.map_err(|e| {
                BridgeError::DbError(format!("Failed to connect to admin database: {}", e))
            })?;

            sqlx::migrate!("./migrations")
                .run(&admin_pool)
                .await
                .map_err(|e| BridgeError::DbError(format!("Failed to run migrations: {}", e)))?;
        } else {
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .map_err(|e| BridgeError::DbError(format!("Failed to run migrations: {}", e)))?;
        }

        info!("Migrations completed successfully");

        Ok(Self { pool })
    }
}

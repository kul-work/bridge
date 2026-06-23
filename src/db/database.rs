use crate::error::BridgeError;
use sqlx::PgPool;
use std::str::FromStr;
use tracing::{error, info};
use uuid::Uuid;

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
        let mut opts = sqlx::postgres::PgConnectOptions::from_str(database_url).map_err(|e| {
            error!(error = %e, "database URL parse failed");
            BridgeError::ConfigError("Failed to parse database URL".to_string())
        })?;

        // Only set SSL cert for SSL-required connections (e.g., Supabase, Neon)
        if database_url.contains("sslmode=require") {
            let cert_path = if database_url.contains("supabase.com") {
                "./certs/prod-ca-2021.crt"
            } else if database_url.contains("neon.tech") {
                "./certs/isrgrootx1.pem"
            } else {
                "./certs/prod-ca-2021.crt" // default fallback
            };
            info!(cert_path = %cert_path, "database TLS root cert configured");
            opts = opts.ssl_root_cert(cert_path);
        }

        let pool = PgPool::connect_with(opts).await.map_err(|e| {
            error!(error = %e, "database pool connect failed");
            BridgeError::DbError(format!("Failed to connect to database: {}", e))
        })?;

        // Run migrations with admin credentials when available, so runtime app role
        // can stay least-privilege.
        if let Some(admin_url) = admin_database_url {
            let admin_opts = sqlx::postgres::PgConnectOptions::from_str(admin_url).map_err(|e| {
                error!(error = %e, "admin database URL parse failed");
                BridgeError::ConfigError("Failed to parse ADMIN_DATABASE_URL".to_string())
            })?;
            let admin_pool = PgPool::connect_with(admin_opts).await.map_err(|e| {
                error!(error = %e, "admin database pool connect failed");
                BridgeError::DbError(format!("Failed to connect to admin database: {}", e))
            })?;

            sqlx::migrate!("./migrations")
                .run(&admin_pool)
                .await
                .map_err(|e| {
                    error!(error = %e, migration_pool = "admin", "database migration failed");
                    BridgeError::DbError(format!("Failed to run migrations: {}", e))
                })?;
        } else {
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .map_err(|e| {
                    error!(error = %e, migration_pool = "runtime", "database migration failed");
                    BridgeError::DbError(format!("Failed to run migrations: {}", e))
                })?;
        }

        info!("Migrations completed successfully");

        Ok(Self { pool })
    }
}

pub(crate) async fn set_local_app_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
) -> Result<(), BridgeError> {
    sqlx::query("SELECT set_config('bridge.current_app_id', $1, true)")
        .bind(app_id.to_string())
        .execute(&mut **tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

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

    /// Test-only constructor that wraps an existing pool without running
    /// migrations, so DB-backed tests can call repository trait methods.
    #[cfg(test)]
    pub(crate) fn from_pool_for_test(pool: PgPool) -> Self {
        Database { pool }
    }

    /// Create a new database connection pool and run migrations
    pub async fn new(
        database_url: &str,
        admin_database_url: Option<&str>,
    ) -> Result<Self, BridgeError> {
        let mut opts = sqlx::postgres::PgConnectOptions::from_str(database_url).map_err(|e| {
            error!(
                signal_class = "alert_signal",
                alert_key = "bridge.db.role_or_rls_failed",
                alert_severity = "page",
                alert_subject = "Bridge database configuration failed",
                error = %e,
                "database URL parse failed"
            );
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
            error!(
                signal_class = "alert_signal",
                alert_key = "bridge.db.readiness_failed",
                alert_severity = "page",
                alert_subject = "Bridge database connection failed",
                error = %e,
                "database pool connect failed"
            );
            BridgeError::DbError(format!("Failed to connect to database: {}", e))
        })?;

        // Run migrations with admin credentials when available, so runtime app role
        // can stay least-privilege.
        if let Some(admin_url) = admin_database_url {
            let admin_opts = sqlx::postgres::PgConnectOptions::from_str(admin_url).map_err(|e| {
                error!(
                    signal_class = "alert_signal",
                    alert_key = "bridge.db.role_or_rls_failed",
                    alert_severity = "page",
                    alert_subject = "Bridge admin database configuration failed",
                    error = %e,
                    "admin database URL parse failed"
                );
                BridgeError::ConfigError("Failed to parse ADMIN_DATABASE_URL".to_string())
            })?;
            let admin_pool = PgPool::connect_with(admin_opts).await.map_err(|e| {
                error!(
                    signal_class = "alert_signal",
                    alert_key = "bridge.db.readiness_failed",
                    alert_severity = "page",
                    alert_subject = "Bridge admin database connection failed",
                    error = %e,
                    "admin database pool connect failed"
                );
                BridgeError::DbError(format!("Failed to connect to admin database: {}", e))
            })?;

            sqlx::migrate!("./migrations")
                .run(&admin_pool)
                .await
                .map_err(|e| {
                    error!(
                        signal_class = "alert_signal",
                        alert_key = "bridge.db.role_or_rls_failed",
                        alert_severity = "page",
                        alert_subject = "Bridge database migration failed",
                        error = %e,
                        migration_pool = "admin",
                        "database migration failed"
                    );
                    BridgeError::DbError(format!("Failed to run migrations: {}", e))
                })?;
        } else {
            let environment = std::env::var("ENVIRONMENT")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(environment.as_str(), "production" | "prod") {
                error!(
                    signal_class = "alert_signal",
                    alert_key = "bridge.db.role_or_rls_failed",
                    alert_severity = "page",
                    alert_subject = "Bridge admin database configuration failed",
                    "ADMIN_DATABASE_URL is required in production"
                );
                return Err(BridgeError::ConfigError(
                    "ADMIN_DATABASE_URL is required in production".to_string(),
                ));
            }

            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .map_err(|e| {
                    error!(
                        signal_class = "alert_signal",
                        alert_key = "bridge.db.role_or_rls_failed",
                        alert_severity = "page",
                        alert_subject = "Bridge database migration failed",
                        error = %e,
                        migration_pool = "runtime",
                        "database migration failed"
                    );
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
        .map_err(|e| {
            error!(
                signal_class = "alert_signal",
                alert_key = "bridge.db.role_or_rls_failed",
                alert_severity = "page",
                alert_subject = "Bridge database app context failed",
                app_id = %app_id,
                error = %e,
                "database app context set failed"
            );
            BridgeError::DbError(e.to_string())
        })?;

    Ok(())
}

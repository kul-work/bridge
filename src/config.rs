use anyhow::Result;
use std::env;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub admin_database_url: Option<String>,
    pub server_addr: String,
    pub server_port: u16,
    pub logging_level: String,
    pub environment: String,
    pub mock_external_apis: bool,
    pub enable_background_jobs: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        // Helper for parsing optional u16 with defaults
        fn parse_u16_env(key: &str, default: u16) -> Result<u16> {
            env::var(key)
                .unwrap_or_else(|_| default.to_string())
                .parse()
                .map_err(|e| anyhow::anyhow!("Failed to parse {} as u16: {}", key, e))
        }

        fn parse_bool_env(key: &str, default: bool) -> Result<bool> {
            let raw = env::var(key).unwrap_or_else(|_| default.to_string());
            match raw.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(anyhow::anyhow!("Failed to parse {} as bool: {}", key, raw)),
            }
        }

        Ok(Config {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgresql://localhost/bridge".to_string()),
            admin_database_url: env::var("ADMIN_DATABASE_URL").ok(),
            server_addr: env::var("SERVER_ADDR")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            server_port: parse_u16_env("PORT", 3000)?,
            logging_level: env::var("LOGGING_LEVEL")
                .unwrap_or_else(|_| "info".to_string()),
            environment: env::var("ENVIRONMENT")
                .unwrap_or_else(|_| "development".to_string()),
            mock_external_apis: parse_bool_env("MOCK_EXTERNAL_APIS", false)?,
            enable_background_jobs: parse_bool_env("ENABLE_BACKGROUND_JOBS", true)?,
        })
    }
}

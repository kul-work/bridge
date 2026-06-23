use anyhow::Result;
use std::{env, path::Path};

pub const API_PAGINATION_LIMIT: i64 = 20;
pub const MAX_PAGINATION_LIMIT: i64 = 100;
pub const DATA_EXPORT_LIMIT: i64 = 100;
pub const ADMIN_WEBHOOK_LIST_LIMIT: i64 = 100;
const ADMIN_TEST_ENV_PATH: &str = "tests/admin/.env";
const ADMIN_TEST_ENV_KEYS: &[&str] = &[
    "ADMIN_CLERK_FRONTEND_API",
    "ADMIN_CLERK_AUTHORIZED_PARTIES",
    "ADMIN_CLERK_ORG_ID",
];

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
        let environment = env::var("ENVIRONMENT")
            .unwrap_or_else(|_| "development".to_string());
        if should_load_admin_test_env(&environment) {
            load_admin_test_env();
        }

        // Helper for parsing optional u16 with defaults
        fn parse_u16_env(key: &str, default: u16) -> Result<u16> {
            env::var(key)
                .unwrap_or_else(|_| default.to_string())
                .parse()
                .map_err(|e| anyhow::anyhow!("Failed to parse {} as u16: {}", key, e))
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
            environment,
            mock_external_apis: parse_bool_env("MOCK_EXTERNAL_APIS", false)?,
            enable_background_jobs: parse_bool_env("ENABLE_BACKGROUND_JOBS", true)?,
        })
    }
}

fn should_load_admin_test_env(environment: &str) -> bool {
    matches!(
        environment.trim().to_ascii_lowercase().as_str(),
        "development" | "dev" | "local" | "test"
    )
}

fn load_admin_test_env() {
    if !Path::new(ADMIN_TEST_ENV_PATH).exists() {
        return;
    }

    let Ok(vars) = dotenvy::from_path_iter(ADMIN_TEST_ENV_PATH) else {
        return;
    };

    for item in vars.flatten() {
        let (key, value) = item;
        if ADMIN_TEST_ENV_KEYS.contains(&key.as_str()) && env::var_os(&key).is_none() {
            env::set_var(key, value);
        }
    }
}

pub fn parse_bool_env(key: &str, default: bool) -> Result<bool> {
    let raw = env::var(key).unwrap_or_else(|_| default.to_string());
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(anyhow::anyhow!("Failed to parse {} as bool: {}", key, raw)),
    }
}

pub fn mock_external_apis_enabled() -> bool {
    parse_bool_env("MOCK_EXTERNAL_APIS", false).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::should_load_admin_test_env;

    #[test]
    fn admin_test_env_loads_only_for_local_environments() {
        assert!(should_load_admin_test_env("development"));
        assert!(should_load_admin_test_env("dev"));
        assert!(should_load_admin_test_env("local"));
        assert!(should_load_admin_test_env("test"));
    }

    #[test]
    fn admin_test_env_does_not_load_for_deployed_environments() {
        assert!(!should_load_admin_test_env("production"));
        assert!(!should_load_admin_test_env("staging"));
    }
}

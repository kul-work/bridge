use anyhow::Result;
use std::{env, path::Path};

pub const API_PAGINATION_LIMIT: i64 = 20;
pub const MAX_PAGINATION_LIMIT: i64 = 100;
pub const DATA_EXPORT_LIMIT: i64 = 100;
pub const ADMIN_WEBHOOK_LIST_LIMIT: i64 = 100;
const ADMIN_TEST_ENV_VAR: &str = "BRIDGE_ADMIN_TEST_ENV";
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
        if let Some(path) = admin_test_env_path(env::var(ADMIN_TEST_ENV_VAR).ok().as_deref()) {
            load_admin_test_env(path);
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

fn admin_test_env_path(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn load_admin_test_env(path: &str) {
    if !Path::new(path).exists() {
        return;
    }

    let Ok(vars) = dotenvy::from_path_iter(path) else {
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
    use super::admin_test_env_path;

    #[test]
    fn admin_test_env_requires_explicit_path() {
        assert_eq!(admin_test_env_path(None), None);
        assert_eq!(admin_test_env_path(Some("")), None);
        assert_eq!(admin_test_env_path(Some("   ")), None);
    }

    #[test]
    fn admin_test_env_accepts_configured_path() {
        assert_eq!(
            admin_test_env_path(Some(" tests/admin/.env ")),
            Some("tests/admin/.env")
        );
    }
}

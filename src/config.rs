use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sqlx::postgres::PgConnectOptions;
use std::{env, net::IpAddr, path::Path, str::FromStr};
use url::Url;

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

    pub fn validate_startup(&self) -> Result<()> {
        let env_lower = self.environment.to_ascii_lowercase();
        if !matches!(env_lower.as_str(), "production" | "prod") {
            return Ok(());
        }

        let errors = self.production_startup_errors(|key| env::var(key).ok());
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Invalid production configuration: {}",
                errors.join("; ")
            ))
        }
    }

    fn production_startup_errors<F>(&self, env_var: F) -> Vec<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut errors = Vec::new();

        if self.mock_external_apis {
            errors.push("MOCK_EXTERNAL_APIS=true is not allowed in production".to_string());
        }

        if env_var("DATABASE_URL").is_none() {
            errors.push("DATABASE_URL must be set in production".to_string());
        } else if PgConnectOptions::from_str(&self.database_url).is_err() {
            errors.push("DATABASE_URL must be a valid PostgreSQL connection string".to_string());
        }

        let publishable_key = trimmed_env(&env_var, "CLERK_PUBLISHABLE_KEY");
        if publishable_key.is_none() {
            errors.push("CLERK_PUBLISHABLE_KEY must be set in production".to_string());
        }

        let admin_issuer = trimmed_env(&env_var, "ADMIN_CLERK_FRONTEND_API")
            .or_else(|| trimmed_env(&env_var, "CLERK_FRONTEND_API"))
            .or_else(|| publishable_key.as_deref().and_then(derive_clerk_issuer_from_publishable_key));
        match admin_issuer {
            Some(ref value) => validate_public_https_url(
                "ADMIN_CLERK_FRONTEND_API/CLERK_FRONTEND_API/CLERK_PUBLISHABLE_KEY",
                value,
                &mut errors,
            ),
            None => errors.push(
                "ADMIN_CLERK_FRONTEND_API, CLERK_FRONTEND_API, or a derivable CLERK_PUBLISHABLE_KEY must be set in production"
                    .to_string(),
            ),
        }

        match trimmed_env(&env_var, "ADMIN_CLERK_AUTHORIZED_PARTIES") {
            Some(value) => {
                let parties: Vec<_> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .collect();
                if parties.is_empty() {
                    errors.push(
                        "ADMIN_CLERK_AUTHORIZED_PARTIES must include at least one origin in production"
                            .to_string(),
                    );
                }
                for party in parties {
                    validate_public_https_url("ADMIN_CLERK_AUTHORIZED_PARTIES", party, &mut errors);
                }
            }
            None => errors.push(
                "ADMIN_CLERK_AUTHORIZED_PARTIES must be set in production".to_string(),
            ),
        }

        errors
    }
}

fn trimmed_env<F>(env_var: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    env_var(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_public_https_url(label: &str, value: &str, errors: &mut Vec<String>) {
    let Ok(url) = Url::parse(value) else {
        errors.push(format!("{} must be a valid URL", label));
        return;
    };

    if url.scheme() != "https" {
        errors.push(format!("{} must use https in production", label));
        return;
    }

    let Some(host) = url.host_str() else {
        errors.push(format!("{} must include a host", label));
        return;
    };

    if is_unsafe_production_host(host) {
        errors.push(format!("{} must not use localhost, private, or test hosts in production", label));
    }
}

fn derive_clerk_issuer_from_publishable_key(publishable_key: &str) -> Option<String> {
    let mut parts = publishable_key.splitn(3, '_');
    if parts.next()? != "pk" {
        return None;
    }
    parts.next()?;
    let encoded_host = parts.next()?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded_host).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let host = decoded.trim_end_matches('$').trim();
    if host.is_empty() {
        return None;
    }

    Some(format!("https://{}", host.trim_end_matches('/')))
}

fn is_unsafe_production_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".test")
        || host.ends_with(".invalid")
        || host.ends_with(".example")
    {
        return true;
    }

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
        Err(_) => false,
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
    use std::collections::HashMap;

    use super::{admin_test_env_path, Config};

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

    fn test_config() -> Config {
        Config {
            database_url: "postgresql://bridge_app:password@db.example.com/appgen".to_string(),
            admin_database_url: None,
            server_addr: "0.0.0.0".to_string(),
            server_port: 3000,
            logging_level: "info".to_string(),
            environment: "production".to_string(),
            mock_external_apis: false,
            enable_background_jobs: true,
        }
    }

    fn env_getter(values: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |key| values.get(key).map(|value| value.to_string())
    }

    #[test]
    fn production_startup_accepts_explicit_admin_boundary() {
        let config = test_config();
        let env = env_getter(HashMap::from([
            ("DATABASE_URL", "postgresql://bridge_app:password@db.example.com/appgen"),
            ("ADMIN_CLERK_FRONTEND_API", "https://admin-clerk.tyde.app"),
            ("CLERK_PUBLISHABLE_KEY", "pk_test_dGVzdC1icmlkZ2UtYWRtaW4uY2xlcmsuYWNjb3VudHMuZGV2JA"),
            ("ADMIN_CLERK_AUTHORIZED_PARTIES", "https://admin.tyde.app"),
        ]));

        assert!(config.production_startup_errors(env).is_empty());
    }

    #[test]
    fn production_startup_accepts_publishable_key_issuer_fallback() {
        let config = test_config();
        let env = env_getter(HashMap::from([
            ("DATABASE_URL", "postgresql://bridge_app:password@db.example.com/appgen"),
            ("CLERK_PUBLISHABLE_KEY", "pk_test_YWRtaW4tdHlkZS5jbGVyay5hY2NvdW50cy5kZXYk"),
            ("ADMIN_CLERK_AUTHORIZED_PARTIES", "https://admin.tyde.app"),
        ]));

        assert!(config.production_startup_errors(env).is_empty());
    }

    #[test]
    fn production_startup_rejects_missing_admin_boundary() {
        let config = test_config();
        let env = env_getter(HashMap::from([
            ("DATABASE_URL", "postgresql://bridge_app:password@db.example.com/appgen"),
        ]));

        let errors = config.production_startup_errors(env);

        assert!(errors.iter().any(|error| error.contains("ADMIN_CLERK_FRONTEND_API")));
        assert!(errors.iter().any(|error| error.contains("CLERK_PUBLISHABLE_KEY")));
        assert!(errors.iter().any(|error| error.contains("ADMIN_CLERK_AUTHORIZED_PARTIES")));
    }

    #[test]
    fn production_startup_rejects_missing_publishable_key_even_with_explicit_issuer() {
        let config = test_config();
        let env = env_getter(HashMap::from([
            ("DATABASE_URL", "postgresql://bridge_app:password@db.example.com/appgen"),
            ("ADMIN_CLERK_FRONTEND_API", "https://admin-clerk.tyde.app"),
            ("ADMIN_CLERK_AUTHORIZED_PARTIES", "https://admin.tyde.app"),
        ]));

        let errors = config.production_startup_errors(env);

        assert!(errors.iter().any(|error| error.contains("CLERK_PUBLISHABLE_KEY")));
    }

    #[test]
    fn production_startup_rejects_unsafe_admin_urls() {
        let config = test_config();
        let env = env_getter(HashMap::from([
            ("DATABASE_URL", "postgresql://bridge_app:password@db.example.com/appgen"),
            ("ADMIN_CLERK_FRONTEND_API", "http://localhost:3000"),
            ("CLERK_PUBLISHABLE_KEY", "pk_test_dGVzdC1icmlkZ2UtYWRtaW4uY2xlcmsuYWNjb3VudHMuZGV2JA"),
            ("ADMIN_CLERK_AUTHORIZED_PARTIES", "https://127.0.0.1"),
        ]));

        let errors = config.production_startup_errors(env);

        assert!(errors.iter().any(|error| error.contains("must use https")));
        assert!(errors.iter().any(|error| error.contains("must not use localhost")));
    }

    #[test]
    fn production_startup_rejects_mock_external_apis() {
        let mut config = test_config();
        config.mock_external_apis = true;
        let env = env_getter(HashMap::from([
            ("DATABASE_URL", "postgresql://bridge_app:password@db.example.com/appgen"),
            ("ADMIN_CLERK_FRONTEND_API", "https://admin-clerk.tyde.app"),
            ("CLERK_PUBLISHABLE_KEY", "pk_test_dGVzdC1icmlkZ2UtYWRtaW4uY2xlcmsuYWNjb3VudHMuZGV2JA"),
            ("ADMIN_CLERK_AUTHORIZED_PARTIES", "https://admin.tyde.app"),
        ]));

        let errors = config.production_startup_errors(env);

        assert!(errors.iter().any(|error| error.contains("MOCK_EXTERNAL_APIS")));
    }
}

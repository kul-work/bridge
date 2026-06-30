use crate::config::{is_production_environment, Config};
use crate::error::BridgeError;
use crate::utils::{diagnostic_hash, scrub_email};
use async_trait::async_trait;
use reqwest::{header::RETRY_AFTER, Client};
use std::{
    env,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tracing::{error, info, warn};
use uuid::Uuid;

const EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS: u64 = 300;
const EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS: u64 = 900;

#[derive(Clone, Copy, Debug, Default)]
pub struct EmailContext<'a> {
    pub email_type: Option<&'a str>,
    pub app_id: Option<Uuid>,
    pub provider: Option<&'a str>,
    pub event_type: Option<&'a str>,
    pub provider_webhook_id: Option<&'a str>,
    pub external_user_id: Option<&'a str>,
    pub subscription_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

fn email_provider_http_failure(status: u16) -> (&'static str, &'static str) {
    match status {
        401 | 403 => (
            "auth_or_permission",
            "Email provider authentication or sender permission failed",
        ),
        429 => ("rate_limited", "Email provider rate limited request"),
        400..=499 => ("request_rejected", "Email provider rejected request"),
        500..=599 => ("provider_unavailable", "Email provider unavailable"),
        _ => (
            "non_success",
            "Email provider returned non-success status",
        ),
    }
}

struct RateLimitCooldown<'a> {
    until: &'a Mutex<Option<Instant>>,
    default_seconds: u64,
    max_seconds: u64,
}

fn email_provider_rate_limit_cooldown_config() -> Result<(u64, u64), BridgeError> {
    let default_seconds = parse_u64_env(
        "EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS",
        EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS,
    )?;
    let max_seconds = parse_u64_env(
        "EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS",
        EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS,
    )?;

    if default_seconds == 0 || max_seconds == 0 {
        return Err(BridgeError::ConfigError(
            "Email provider cooldown seconds must be greater than zero".to_string(),
        ));
    }
    if default_seconds > max_seconds {
        return Err(BridgeError::ConfigError(
            "EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS must be less than or equal to EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS".to_string(),
        ));
    }

    Ok((default_seconds, max_seconds))
}

fn parse_u64_env(key: &str, default: u64) -> Result<u64, BridgeError> {
    env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .parse::<u64>()
        .map_err(|err| BridgeError::ConfigError(format!("Failed to parse {key} as u64: {err}")))
}

fn email_provider_rate_limit_cooldown_seconds(
    retry_after: Option<&str>,
    default_seconds: u64,
    max_seconds: u64,
) -> u64 {
    retry_after
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.min(max_seconds))
        .unwrap_or(default_seconds)
}

fn start_provider_rate_limit_cooldown(cooldown_until: &Mutex<Option<Instant>>, seconds: u64) {
    let mut cooldown_until = cooldown_until
        .lock()
        .expect("email provider cooldown lock poisoned");
    *cooldown_until = Some(Instant::now() + Duration::from_secs(seconds));
}

fn provider_rate_limit_cooldown_remaining_seconds(
    cooldown_until: &Mutex<Option<Instant>>,
) -> Option<u64> {
    let mut cooldown_until = cooldown_until
        .lock()
        .expect("email provider cooldown lock poisoned");
    let until = (*cooldown_until)?;
    let now = Instant::now();
    if until <= now {
        *cooldown_until = None;
        return None;
    }

    Some(until.duration_since(now).as_secs().max(1))
}

fn scrub_emails_and_keys(input: &str) -> String {
    let scrubbed = scrub_email(input);
    let chars = scrubbed.chars().collect::<Vec<char>>();
    let prefixes = ["re_", "sk_", "pk_", "secret_", "token_"];
    let mut output = String::new();
    let mut index = 0;

    while index < chars.len() {
        let matched_prefix = prefixes.iter().find(|prefix| {
            let prefix_chars = prefix.chars().collect::<Vec<char>>();
            chars[index..].starts_with(&prefix_chars)
        });

        if let Some(prefix) = matched_prefix {
            let mut end = index;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric() || chars[end] == '_' || chars[end] == '-')
            {
                end += 1;
            }

            if end - index > prefix.len() + 1 {
                output.push_str("[redacted_key]");
                index = end;
                continue;
            }
        }

        output.push(chars[index]);
        index += 1;
    }

    output
}

fn recipient_hash(email: &str) -> String {
    diagnostic_hash(&email.to_ascii_lowercase())
}

fn external_user_id_hash(external_user_id: Option<&str>) -> Option<String> {
    external_user_id.map(diagnostic_hash)
}

async fn send_via_provider(
    api_label: &str,
    endpoint: &str,
    api_key: &str,
    payload: serde_json::Value,
    context: EmailContext<'_>,
    recipient_hash: &str,
    cooldown: Option<RateLimitCooldown<'_>>,
) -> Result<(), BridgeError> {
    let external_user_id_hash = external_user_id_hash(context.external_user_id);
    let client = Client::new();
    let response = match client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await {
            Ok(response) => response,
            Err(e) => {
                error!(
                    provider = api_label,
                    email_type = context.email_type,
                    app_id = ?context.app_id,
                    payment_provider = context.provider,
                    event_type = context.event_type,
                    provider_webhook_id = context.provider_webhook_id,
                    external_user_id_hash = external_user_id_hash.as_deref(),
                    subscription_id = context.subscription_id,
                    email_idempotency_key = context.idempotency_key,
                    recipient_hash,
                    error_kind = "request_failed",
                    error_msg = %scrub_emails_and_keys(&e.to_string()),
                    "Email provider request failed"
                );
                return Err(BridgeError::InternalServerError(format!("Failed to send email via {}: {}", api_label, e)));
            }
        };

    if !response.status().is_success() {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        let (error_kind, message) = email_provider_http_failure(status.as_u16());
        let cooldown_seconds = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            if let Some(cooldown) = cooldown {
                let seconds = email_provider_rate_limit_cooldown_seconds(
                    Some(&retry_after),
                    cooldown.default_seconds,
                    cooldown.max_seconds,
                );
                start_provider_rate_limit_cooldown(cooldown.until, seconds);
                seconds
            } else {
                0
            }
        } else {
            0
        };
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            log_email_auth_or_permission_failure(api_label, Some(status.as_u16()), "provider_rejected", context);
        }
        error!(
            provider = api_label,
            email_type = context.email_type,
            app_id = ?context.app_id,
            payment_provider = context.provider,
            event_type = context.event_type,
            provider_webhook_id = context.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id = context.subscription_id,
            email_idempotency_key = context.idempotency_key,
            recipient_hash,
            provider_status = status.as_u16(),
            error_kind,
            retry_after = %retry_after,
            cooldown_seconds,
            error_body = %scrub_emails_and_keys(&error_text),
            "Email provider returned non-success status"
        );
        return Err(BridgeError::InternalServerError(message.into()));
    }

    info!(
        provider = api_label,
        email_type = context.email_type,
        app_id = ?context.app_id,
        payment_provider = context.provider,
        event_type = context.event_type,
        provider_webhook_id = context.provider_webhook_id,
        external_user_id_hash = external_user_id_hash.as_deref(),
        subscription_id = context.subscription_id,
        email_idempotency_key = context.idempotency_key,
        recipient_hash,
        "Email provider accepted message"
    );

    Ok(())
}

fn log_email_auth_or_permission_failure(
    provider: &str,
    http_status: Option<u16>,
    failure: &'static str,
    context: EmailContext<'_>,
) {
    let external_user_id_hash = external_user_id_hash(context.external_user_id);
    error!(
        signal_class = "alert_signal",
        alert_key = "bridge.email.auth_or_permission_failed",
        alert_severity = "ticket",
        alert_subject = "Email provider auth or permission failure",
        provider,
        email_type = context.email_type,
        app_id = ?context.app_id,
        payment_provider = context.provider,
        event_type = context.event_type,
        provider_webhook_id = context.provider_webhook_id,
        external_user_id_hash = external_user_id_hash.as_deref(),
        subscription_id = context.subscription_id,
        email_idempotency_key = context.idempotency_key,
        http_status,
        failure,
        "Email provider auth or permission failure"
    );
}

#[async_trait]
pub trait EmailService: Send + Sync {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), BridgeError>;
    async fn send_email_with_context(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        context: EmailContext<'_>,
    ) -> Result<(), BridgeError> {
        let _ = context;
        self.send_email(to, subject, body).await
    }
}

pub struct MockEmailService;

#[async_trait]
impl EmailService for MockEmailService {
    async fn send_email(&self, _to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
        self.send_email_with_context(_to, subject, body, EmailContext::default()).await
    }

    async fn send_email_with_context(
        &self,
        _to: &str,
        subject: &str,
        body: &str,
        context: EmailContext<'_>,
    ) -> Result<(), BridgeError> {
        let external_user_id_hash = external_user_id_hash(context.external_user_id);
        warn!(
            email_type = context.email_type,
            app_id = ?context.app_id,
            payment_provider = context.provider,
            event_type = context.event_type,
            provider_webhook_id = context.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id = context.subscription_id,
            email_idempotency_key = context.idempotency_key,
            email_subject = %scrub_emails_and_keys(subject),
            email_body = %scrub_emails_and_keys(body),
            "MOCK EMAIL SENT"
        );
        Ok(())
    }
}

pub struct ClerkEmailService {
    api_key: String,
}

impl ClerkEmailService {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl EmailService for ClerkEmailService {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
        self.send_email_with_context(to, subject, body, EmailContext::default()).await
    }

    async fn send_email_with_context(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        context: EmailContext<'_>,
    ) -> Result<(), BridgeError> {
        let recipient_hash = recipient_hash(to);
        let external_user_id_hash = external_user_id_hash(context.external_user_id);
        info!(
            provider = "clerk",
            email_type = context.email_type,
            app_id = ?context.app_id,
            payment_provider = context.provider,
            event_type = context.event_type,
            provider_webhook_id = context.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id = context.subscription_id,
            email_idempotency_key = context.idempotency_key,
            recipient_hash = %recipient_hash,
            "Sending lifecycle email"
        );

        let payload = serde_json::json!({
            "email_address": to,
            "message": body,
            "subject": subject
        });

        send_via_provider(
            "clerk",
            "https://api.clerk.com/v1/emails",
            &self.api_key,
            payload,
            context,
            &recipient_hash,
            None,
        )
        .await
    }
}

pub struct ResendEmailService {
    api_key: String,
    from_email: String,
    provider_default_rate_limit_cooldown_seconds: u64,
    provider_max_rate_limit_cooldown_seconds: u64,
    provider_rate_limit_cooldown_until: Mutex<Option<Instant>>,
}

impl ResendEmailService {
    pub fn new(
        api_key: String,
        from_email: String,
        provider_default_rate_limit_cooldown_seconds: u64,
        provider_max_rate_limit_cooldown_seconds: u64,
    ) -> Self {
        Self {
            api_key,
            from_email,
            provider_default_rate_limit_cooldown_seconds,
            provider_max_rate_limit_cooldown_seconds,
            provider_rate_limit_cooldown_until: Mutex::new(None),
        }
    }
}

#[async_trait]
impl EmailService for ResendEmailService {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
        self.send_email_with_context(to, subject, body, EmailContext::default()).await
    }

    async fn send_email_with_context(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        context: EmailContext<'_>,
    ) -> Result<(), BridgeError> {
        let recipient_hash = recipient_hash(to);
        let external_user_id_hash = external_user_id_hash(context.external_user_id);
        if let Some(cooldown_remaining_seconds) =
            provider_rate_limit_cooldown_remaining_seconds(&self.provider_rate_limit_cooldown_until)
        {
            warn!(
                provider = "resend",
                email_type = context.email_type,
                app_id = ?context.app_id,
                payment_provider = context.provider,
                event_type = context.event_type,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = external_user_id_hash.as_deref(),
                subscription_id = context.subscription_id,
                email_idempotency_key = context.idempotency_key,
                recipient_hash = %recipient_hash,
                error_kind = "provider_rate_limit_cooldown",
                cooldown_remaining_seconds,
                "Email skipped because email provider is rate limited"
            );
            return Err(BridgeError::InternalServerError(
                "Email provider rate limited request".to_string(),
            ));
        }

        info!(
            provider = "resend",
            email_type = context.email_type,
            app_id = ?context.app_id,
            payment_provider = context.provider,
            event_type = context.event_type,
            provider_webhook_id = context.provider_webhook_id,
            external_user_id_hash = external_user_id_hash.as_deref(),
            subscription_id = context.subscription_id,
            email_idempotency_key = context.idempotency_key,
            recipient_hash = %recipient_hash,
            "Sending lifecycle email"
        );

        let payload = serde_json::json!({
            "from": self.from_email,
            "to": to,
            "subject": subject,
            "html": body
        });

        send_via_provider(
            "resend",
            "https://api.resend.com/emails",
            &self.api_key,
            payload,
            context,
            &recipient_hash,
            Some(RateLimitCooldown {
                until: &self.provider_rate_limit_cooldown_until,
                default_seconds: self.provider_default_rate_limit_cooldown_seconds,
                max_seconds: self.provider_max_rate_limit_cooldown_seconds,
            }),
        )
        .await
    }
}

fn build_email_service(config: &Config) -> Result<Arc<dyn EmailService>, BridgeError> {
    build_email_service_for_environment(&config.environment)
}

pub fn configured_email_provider() -> String {
    std::env::var("EMAIL_PROVIDER")
        .unwrap_or_else(|_| "mock".to_string())
        .trim()
        .to_ascii_lowercase()
}

fn build_email_service_for_environment(environment: &str) -> Result<Arc<dyn EmailService>, BridgeError> {
    let provider = configured_email_provider();
    let is_production = is_production_environment(environment);
    let (provider_default_rate_limit_cooldown_seconds, provider_max_rate_limit_cooldown_seconds) =
        email_provider_rate_limit_cooldown_config()?;

    let service: Box<dyn EmailService> = match provider.as_str() {
        "clerk" => {
            let api_key = std::env::var("CLERK_SECRET_KEY").unwrap_or_default();
            if api_key.is_empty() {
                if is_production {
                    log_email_auth_or_permission_failure("clerk", None, "missing_api_key", EmailContext::default());
                    return Err(BridgeError::ConfigError(
                        "EMAIL_PROVIDER=clerk requires CLERK_SECRET_KEY in production".to_string(),
                    ));
                }
                warn!("CLERK_SECRET_KEY not set, falling back to MockEmailService");
                Box::new(MockEmailService)
            } else {
                Box::new(ClerkEmailService::new(api_key))
            }
        }
        "resend" => {
            let api_key = std::env::var("RESEND_API_KEY").unwrap_or_default();
            let from_email = std::env::var("APP_EMAIL_FROM")
                .unwrap_or_else(|_| "noreply@bridge.local".to_string());
            if api_key.is_empty() {
                if is_production {
                    log_email_auth_or_permission_failure("resend", None, "missing_api_key", EmailContext::default());
                    return Err(BridgeError::ConfigError(
                        "EMAIL_PROVIDER=resend requires RESEND_API_KEY in production".to_string(),
                    ));
                }
                warn!("RESEND_API_KEY not set, falling back to MockEmailService");
                Box::new(MockEmailService)
            } else {
                Box::new(ResendEmailService::new(
                    api_key,
                    from_email,
                    provider_default_rate_limit_cooldown_seconds,
                    provider_max_rate_limit_cooldown_seconds,
                ))
            }
        }
        "mock" => Box::new(MockEmailService),
        _ => {
            if is_production {
                return Err(BridgeError::ConfigError(format!(
                    "Invalid EMAIL_PROVIDER '{}' in production; use clerk or resend",
                    provider
                )));
            }
            Box::new(MockEmailService)
        }
    };

    Ok(Arc::from(service))
}

static EMAIL_SERVICE: OnceLock<Arc<dyn EmailService>> = OnceLock::new();

pub fn init_email_service(config: &Config) -> Result<Arc<dyn EmailService>, BridgeError> {
    if let Some(existing) = EMAIL_SERVICE.get() {
        return Ok(existing.clone());
    }

    let service = build_email_service(config)?;
    let _ = EMAIL_SERVICE.set(service.clone());
    Ok(service)
}

pub fn get_email_service() -> Arc<dyn EmailService> {
    EMAIL_SERVICE
        .get_or_init(|| {
            let environment = std::env::var("ENVIRONMENT")
                .unwrap_or_else(|_| "development".to_string());

            email_service_or_mock_for_environment(
                &environment,
                build_email_service_for_environment(&environment),
            )
        })
        .clone()
}

fn email_service_or_mock_for_environment(
    environment: &str,
    service: Result<Arc<dyn EmailService>, BridgeError>,
) -> Arc<dyn EmailService> {
    match service {
        Ok(service) => service,
        Err(err) if is_production_environment(environment) => {
            panic!("Email service initialization failed in production fallback: {}", err);
        }
        Err(err) => {
            warn!(
                error = %err,
                "Email service initialization failed, falling back to MockEmailService"
            );
            Arc::new(MockEmailService)
        }
    }
}

pub async fn send_email(to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
    get_email_service().send_email(to, subject, body).await
}

#[allow(dead_code)]
pub fn send_email_mock(_to: &str, subject: &str, body: &str) {
    warn!(
        "MOCK EMAIL SENT - subject: {}, body: {}",
        scrub_emails_and_keys(subject),
        scrub_emails_and_keys(body)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvRestore {
        key: &'static str,
        value: Option<String>,
    }

    impl EnvRestore {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key,
                value: previous,
            }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self {
                key,
                value: previous,
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            if let Some(value) = self.value.as_ref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn scrub_emails_and_keys_redacts_email_and_provider_keys() {
        assert_eq!(
            scrub_emails_and_keys("failed for user@example.com key re_123456 token_abcdef"),
            "failed for [redacted_email] key [redacted_key] [redacted_key]"
        );
        assert_eq!(
            scrub_emails_and_keys(r#"{"message":"bad key re_123456"}"#),
            r#"{"message":"bad key [redacted_key]"}"#
        );
    }

    #[test]
    fn email_provider_http_failure_classifies_rate_limit() {
        assert_eq!(email_provider_http_failure(429).0, "rate_limited");
        assert_eq!(
            email_provider_http_failure(403).0,
            "auth_or_permission"
        );
    }

    #[test]
    fn email_provider_rate_limit_cooldown_uses_retry_after_with_bounds() {
        assert_eq!(
            email_provider_rate_limit_cooldown_seconds(None, 300, 900),
            300
        );
        assert_eq!(
            email_provider_rate_limit_cooldown_seconds(Some("120"), 300, 900),
            120
        );
        assert_eq!(
            email_provider_rate_limit_cooldown_seconds(Some("9999"), 300, 900),
            900
        );
    }

    #[test]
    fn fallback_email_service_panics_on_production_config_error() {
        let result = std::panic::catch_unwind(|| {
            email_service_or_mock_for_environment(
                "production",
                Err(BridgeError::ConfigError("bad email config".to_string())),
            );
        });

        assert!(result.is_err());
    }

    #[test]
    fn email_service_builder_treats_trimmed_production_as_production() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _provider = EnvRestore::set("EMAIL_PROVIDER", "resend");
        let _api_key = EnvRestore::remove("RESEND_API_KEY");

        let result = build_email_service_for_environment(" Production ");

        assert!(matches!(result, Err(BridgeError::ConfigError(_))));
    }

    #[test]
    fn configured_email_provider_trims_and_normalizes_case() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _provider = EnvRestore::set("EMAIL_PROVIDER", " Resend ");

        assert_eq!(configured_email_provider(), "resend");
    }

    #[test]
    fn recipient_hash_is_stable_and_case_insensitive() {
        assert_eq!(
            recipient_hash("User@Example.com"),
            recipient_hash("user@example.com")
        );
        assert_ne!(recipient_hash("user@example.com"), "user@example.com");
    }

    #[test]
    fn external_user_id_hash_redacts_raw_identifier() {
        assert_ne!(
            external_user_id_hash(Some("user_123")).as_deref(),
            Some("user_123")
        );
        assert_eq!(external_user_id_hash(None), None);
    }
}

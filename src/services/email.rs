use crate::config::Config;
use crate::error::BridgeError;
use async_trait::async_trait;
use reqwest::Client;
use std::sync::{Arc, OnceLock};
use tracing::{error, info, warn};

async fn send_via_provider(
    api_label: &str,
    endpoint: &str,
    api_key: &str,
    payload: serde_json::Value,
) -> Result<(), BridgeError> {
    let client = Client::new();
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            BridgeError::InternalServerError(format!("Failed to send email via {}: {}", api_label, e))
        })?;

    if !response.status().is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        error!("{} API error: {}", api_label, error_text);
        return Err(BridgeError::InternalServerError("Email send failed".into()));
    }

    Ok(())
}

#[async_trait]
pub trait EmailService: Send + Sync {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), BridgeError>;
}

pub struct MockEmailService;

#[async_trait]
impl EmailService for MockEmailService {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
        warn!(
            "MOCK EMAIL SENT - to: {}, subject: {}, body: {}",
            to,
            subject,
            body
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
        info!("Sending email via Clerk to: {}", to);

        let payload = serde_json::json!({
            "email_address": to,
            "message": body,
            "subject": subject
        });

        send_via_provider(
            "Clerk",
            "https://api.clerk.com/v1/emails",
            &self.api_key,
            payload,
        )
        .await
    }
}

pub struct ResendEmailService {
    api_key: String,
    from_email: String,
}

impl ResendEmailService {
    pub fn new(api_key: String, from_email: String) -> Self {
        Self { api_key, from_email }
    }
}

#[async_trait]
impl EmailService for ResendEmailService {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
        info!("Sending email via Resend to: {}", to);

        let payload = serde_json::json!({
            "from": self.from_email,
            "to": to,
            "subject": subject,
            "html": body
        });

        send_via_provider(
            "Resend",
            "https://api.resend.com/emails",
            &self.api_key,
            payload,
        )
        .await
    }
}

fn build_email_service(config: &Config) -> Result<Arc<dyn EmailService>, BridgeError> {
    let provider = std::env::var("EMAIL_PROVIDER").unwrap_or_else(|_| "mock".to_string());
    let environment = config.environment.to_ascii_lowercase();
    let is_production = environment == "production" || environment == "prod";

    let service: Box<dyn EmailService> = match provider.as_str() {
        "clerk" => {
            let api_key = std::env::var("CLERK_SECRET_KEY").unwrap_or_default();
            if api_key.is_empty() {
                if is_production {
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
                    return Err(BridgeError::ConfigError(
                        "EMAIL_PROVIDER=resend requires RESEND_API_KEY in production".to_string(),
                    ));
                }
                warn!("RESEND_API_KEY not set, falling back to MockEmailService");
                Box::new(MockEmailService)
            } else {
                Box::new(ResendEmailService::new(api_key, from_email))
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
            let fallback_config = Config {
                database_url: String::new(),
                admin_database_url: None,
                server_addr: "0.0.0.0".to_string(),
                server_port: 3000,
                logging_level: "info".to_string(),
                environment: std::env::var("ENVIRONMENT")
                    .unwrap_or_else(|_| "development".to_string()),
                mock_external_apis: false,
                enable_background_jobs: true,
            };

            build_email_service(&fallback_config).unwrap_or_else(|_| Arc::new(MockEmailService))
        })
        .clone()
}

pub async fn send_email(to: &str, subject: &str, body: &str) -> Result<(), BridgeError> {
    get_email_service().send_email(to, subject, body).await
}

#[allow(dead_code)]
pub fn send_email_mock(to: &str, subject: &str, body: &str) {
    warn!(
        "MOCK EMAIL SENT - to: {}, subject: {}, body: {}",
        to,
        subject,
        body
    );
}

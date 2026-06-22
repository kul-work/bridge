use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use tracing::error;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

/// AppError enum compatible with payment provider services
/// Used by PaymentProvider trait implementations
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Payment provider error: {0}")]
    PaymentProviderError(String),

    #[error("Webhook verification failed")]
    WebhookVerificationFailed,

    #[error("Webhook parse error")]
    WebhookParseError,

    #[error("Subscription not found")]
    SubscriptionNotFound,

    #[error("Webhook signature verification failed: {0}")]
    WebhookSignatureVerificationFailed(String),

    #[error("Webhook base64 decoding failed: {0}")]
    WebhookBase64DecodingFailed(String),

    #[error("Webhook payload parsing failed: {0}")]
    WebhookPayloadParsingFailed(String),

    #[error("Webhook payload invalid: {0}")]
    WebhookPayloadInvalid(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Internal server error: {0}")]
    InternalServerError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("Database error: {0}")]
    DbError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Unauthorized: {0}")]
    UnauthorizedError(String),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Webhook error: {0}")]
    WebhookError(String),

    #[error("Internal server error: {0}")]
    InternalServerError(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Provider not configured: {0}")]
    ProviderNotConfigured(String),

    #[error("Database operation failed: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Fraud detected: {0}")]
    FraudDetected(String),

    #[error("App disabled: {0}")]
    AppDisabled(String),

    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            BridgeError::DbError(msg) => {
                error!("Database error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    format!("Database error: {}", msg),
                )
            }
            BridgeError::ValidationError(msg) => (
                StatusCode::BAD_REQUEST,
                "validation_error",
                msg.clone(),
            ),
            BridgeError::UnauthorizedError(msg) => {
                error!("Unauthorized: {}", msg);
                (
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    msg.clone(),
                )
            }
            BridgeError::ProviderError(msg) => {
                error!("Provider error: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    "provider_error",
                    msg.clone(),
                )
            }
            BridgeError::WebhookError(msg) => {
                error!("Webhook error: {}", msg);
                (
                    StatusCode::BAD_REQUEST,
                    "webhook_error",
                    msg.clone(),
                )
            }
            BridgeError::InternalServerError(msg) => {
                error!("Internal server error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_server_error",
                    "An internal server error occurred.".to_string(),
                )
            }
            BridgeError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                "bad_request",
                msg.clone(),
            ),
            BridgeError::Forbidden(msg) => (
                StatusCode::FORBIDDEN,
                "forbidden",
                msg.clone(),
            ),
            BridgeError::Conflict(msg) => (
                StatusCode::CONFLICT,
                "conflict",
                msg.clone(),
            ),
            BridgeError::ConfigError(msg) => {
                error!("Configuration error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "config_error",
                    format!("Configuration error: {}", msg),
                )
            }
            BridgeError::ProviderNotConfigured(msg) => {
                error!("Provider not configured: {}", msg);
                (
                    StatusCode::BAD_REQUEST,
                    "provider_not_configured",
                    msg.clone(),
                )
            }
            BridgeError::DatabaseError(e) => {
                error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "Database error occurred.".to_string(),
                )
            }
            BridgeError::FraudDetected(msg) => {
                error!("Fraud detected: {}", msg);
                (
                    StatusCode::CONFLICT,
                    "fraud_detected",
                    msg.clone(),
                )
            }
            BridgeError::AppDisabled(msg) => {
                error!("App disabled: {}", msg);
                (
                    StatusCode::FORBIDDEN,
                    "app_disabled",
                    msg.clone(),
                )
            }
            BridgeError::SubscriptionNotFound(msg) => (
                StatusCode::NOT_FOUND,
                "subscription_not_found",
                msg.clone(),
            ),
        };

        let error_response = ErrorResponse {
            error: error_code.to_string(),
            message,
        };

        (status, Json(error_response)).into_response()
    }
}

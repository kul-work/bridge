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
    pub code: String,
    pub details: serde_json::Value,
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

#[allow(dead_code)]
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

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Database operation failed: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            BridgeError::DbError(msg) => {
                error!("Database error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "DB_ERROR",
                    format!("Database error: {}", msg),
                )
            }
            BridgeError::ValidationError(msg) => (
                StatusCode::BAD_REQUEST,
                "VALIDATION_ERROR",
                format!("Validation error: {}", msg),
            ),
            BridgeError::UnauthorizedError(msg) => {
                error!("Unauthorized: {}", msg);
                (
                    StatusCode::UNAUTHORIZED,
                    "UNAUTHORIZED",
                    format!("Unauthorized: {}", msg),
                )
            }
            BridgeError::ProviderError(msg) => {
                error!("Provider error: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    "PROVIDER_ERROR",
                    format!("Provider error: {}", msg),
                )
            }
            BridgeError::WebhookError(msg) => {
                error!("Webhook error: {}", msg);
                (
                    StatusCode::BAD_REQUEST,
                    "WEBHOOK_ERROR",
                    format!("Webhook error: {}", msg),
                )
            }
            BridgeError::InternalServerError(msg) => {
                error!("Internal server error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_SERVER_ERROR",
                    "An internal server error occurred.".to_string(),
                )
            }
            BridgeError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                msg.clone(),
            ),
            BridgeError::ConfigError(msg) => {
                error!("Configuration error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "CONFIG_ERROR",
                    format!("Configuration error: {}", msg),
                )
            }
            BridgeError::DatabaseError(e) => {
                error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "DATABASE_ERROR",
                    "Database error occurred.".to_string(),
                )
            }
        };

        let error_response = ErrorResponse {
            error: message,
            code: error_code.to_string(),
            details: serde_json::json!({}),
        };

        (status, Json(error_response)).into_response()
    }
}

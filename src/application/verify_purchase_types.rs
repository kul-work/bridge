use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::BridgeError;

#[derive(Debug, Deserialize)]
pub struct VerifyPurchaseRequest {
    pub external_user_id: String,
    pub provider: String,
    pub subscription_id: String,
    pub purchase_token: String,
    pub product_type: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyPurchaseResponse {
    pub status: String,
    pub subscription_id: String,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
    pub amount_cents: Option<i32>,
    pub is_new: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfuscated_account_id: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum ProductType {
    Subscription,
    OneTimeProduct,
}

impl ProductType {
    pub(crate) fn parse(raw: &str) -> Result<Self, BridgeError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "subscription" | "sub" | "subs" => Ok(Self::Subscription),
            "one_time" | "one-time" | "inapp" => Ok(Self::OneTimeProduct),
            _ => Err(BridgeError::ValidationError(
                "product_type must be 'subscription' or 'one_time'".to_string(),
            )),
        }
    }

    pub(crate) fn is_subscription(self) -> bool {
        matches!(self, Self::Subscription)
    }

    pub(crate) fn callback_event_type(self) -> &'static str {
        match self {
            Self::Subscription => "subscription.activated",
            Self::OneTimeProduct => "purchase.one_time",
        }
    }

    pub(crate) fn payment_status(self, status: &str) -> &'static str {
        if status == "pending" {
            "pending"
        } else {
            "success"
        }
    }

    pub(crate) fn callback_status(self, status: &str) -> String {
        match self {
            Self::Subscription => status.to_string(),
            Self::OneTimeProduct if status == "pending" => "pending".to_string(),
            Self::OneTimeProduct => "completed".to_string(),
        }
    }
}

pub(crate) enum VerificationOutcome {
    Verified(VerifiedPurchase),
    LinkingRequired { obfuscated_account_id: String },
}

pub(crate) struct VerifiedPurchase {
    pub(crate) status: String,
    pub(crate) current_period_end: Option<DateTime<Utc>>,
    pub(crate) auto_renewing: Option<bool>,
    pub(crate) amount_cents: Option<i32>,
    pub(crate) payment_state: Option<i32>,
    pub(crate) acknowledgement: PaymentAcknowledgement,
    pub(crate) obfuscated_account_id: Option<String>,
    pub(crate) resubscribe_obfuscated_account_id: Option<String>,
}

pub(crate) enum PaymentAcknowledgement {
    NotApplicable,
    Pending,
    AlreadyAcknowledged,
}

pub(crate) struct VerifyPurchaseCallback<'a> {
    pub(crate) request: &'a VerifyPurchaseRequest,
    pub(crate) resolved_external_user_id: &'a str,
    pub(crate) product_type: ProductType,
    pub(crate) status: &'a str,
    pub(crate) current_period_end: Option<&'a str>,
    pub(crate) auto_renewing: Option<bool>,
    pub(crate) amount_cents: Option<i32>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifyPurchaseSubscriptionSnapshot {
    pub external_user_id: String,
    pub subscription_id: String,
    pub provider: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub provider_customer_id: Option<String>,
}

pub(crate) struct VerifyPurchaseCommitRequest<'a> {
    pub app_id: uuid::Uuid,
    pub resolved_external_user_id: &'a str,
    pub provider: &'a str,
    pub subscription_id: &'a str,
    pub purchase_token: &'a str,
    pub subscription_status: &'a str,
    pub payment_status: &'a str,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub provider_customer_id: Option<&'a str>,
    pub google_obfuscated_account_id: Option<&'a str>,
    pub amount_cents: i32,
    pub event_time_ms: i64,
    pub is_subscription: bool,
}

pub(crate) struct VerifyPurchaseCommitResult {
    pub subscription: Option<VerifyPurchaseSubscriptionSnapshot>,
}

pub(crate) fn compute_obfuscated_id_hash(external_user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(external_user_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

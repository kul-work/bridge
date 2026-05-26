use sha2::{Digest, Sha256};

use crate::application::checkout_types::CheckoutRedirectUrls;
use crate::error::BridgeError;

/// Format integer cents as a dollar string (e.g. 999 -> "9.99", 1000 -> "10.00")
pub(crate) fn format_cents_as_dollars(cents: i32) -> String {
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let remainder = abs % 100;
    let formatted = format!("{}.{:02}", dollars, remainder);
    if cents < 0 {
        format!("-{}", formatted)
    } else {
        formatted
    }
}

pub(crate) fn compute_request_fingerprint(
    external_user_id: &str,
    email: &str,
    provider: &str,
    product_id: &str,
    product_type: Option<&str>,
) -> Result<String, BridgeError> {
    let normalized_payload = serde_json::json!({
        "external_user_id": external_user_id,
        "email": email,
        "provider": provider,
        "product_id": product_id,
        "product_type": product_type,
    });

    let body = serde_json::to_vec(&normalized_payload)
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to serialize checkout payload: {}", e)))?;
    let mut hasher = Sha256::new();
    hasher.update(body);
    Ok(hex::encode(hasher.finalize()))
}

/// Maximum total email length per RFC 5321.
const EMAIL_MAX_LENGTH: usize = 254;
/// Maximum local-part length per RFC 5321.
const EMAIL_LOCAL_MAX_LENGTH: usize = 64;

/// Lightweight email format and length validation.
/// Prevents malformed addresses and header-injection characters from
/// reaching payment providers.
pub(crate) fn validate_email_format(email: &str) -> Result<(), BridgeError> {
    if email.len() > EMAIL_MAX_LENGTH {
        return Err(BridgeError::ValidationError(format!(
            "email exceeds maximum length of {} characters",
            EMAIL_MAX_LENGTH
        )));
    }

    if email.chars().any(|c| c.is_control()) {
        return Err(BridgeError::ValidationError(
            "email contains invalid characters".to_string(),
        ));
    }

    let at_count = email.matches('@').count();
    if at_count != 1 {
        return Err(BridgeError::ValidationError(
            "email must contain exactly one '@'".to_string(),
        ));
    }

    let (local, domain) = email.split_once('@').unwrap();

    if local.is_empty() {
        return Err(BridgeError::ValidationError(
            "email local part before '@' is empty".to_string(),
        ));
    }

    if local.len() > EMAIL_LOCAL_MAX_LENGTH {
        return Err(BridgeError::ValidationError(format!(
            "email local part exceeds maximum length of {} characters",
            EMAIL_LOCAL_MAX_LENGTH
        )));
    }

    if domain.is_empty() {
        return Err(BridgeError::ValidationError(
            "email domain after '@' is empty".to_string(),
        ));
    }

    if !domain.contains('.') {
        return Err(BridgeError::ValidationError(
            "email domain must contain at least one '.'".to_string(),
        ));
    }

    Ok(())
}

pub(crate) fn normalize_required_field(
    value: &str,
    field_name: &str,
) -> Result<String, BridgeError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(BridgeError::ValidationError(format!("{} is required", field_name)));
    }

    Ok(normalized.to_string())
}

pub(crate) fn normalize_provider_name(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "app_store" => "apple".to_string(),
        normalized => normalized.to_string(),
    }
}

pub(crate) fn resolve_checkout_redirect_urls(app_url: Option<&str>) -> CheckoutRedirectUrls {
    let base_url = app_url
        .map(|value| value.trim_end_matches('/').to_string())
        .unwrap_or_else(|| "http://localhost:3000".to_string());

    CheckoutRedirectUrls {
        success_url: base_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_email_accepts_valid() {
        assert!(validate_email_format("user@example.com").is_ok());
        assert!(validate_email_format("a@b.co").is_ok());
        assert!(validate_email_format("user+tag@domain.org").is_ok());
    }

    #[test]
    fn validate_email_rejects_missing_at() {
        let err = validate_email_format("userexample.com").unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("'@'")));
    }

    #[test]
    fn validate_email_rejects_multiple_at() {
        let err = validate_email_format("a@@b.com").unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("'@'")));
    }

    #[test]
    fn validate_email_rejects_empty_local() {
        let err = validate_email_format("@domain.com").unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("local part")));
    }

    #[test]
    fn validate_email_rejects_empty_domain() {
        let err = validate_email_format("user@").unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("domain")));
    }

    #[test]
    fn validate_email_rejects_domain_without_dot() {
        let err = validate_email_format("user@localhost").unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("'.'")));
    }

    #[test]
    fn validate_email_rejects_control_characters() {
        let err = validate_email_format("user\n@example.com").unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("invalid characters")));
    }

    #[test]
    fn validate_email_rejects_excessive_length() {
        let long_local = "a".repeat(65);
        let email = format!("{}@b.com", long_local);
        let err = validate_email_format(&email).unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("local part")));
    }

    #[test]
    fn validate_email_rejects_total_length_exceeding_254() {
        let local = "a".repeat(64);
        let domain = "b".repeat(191); // 64 + 1(@) + 191 = 256
        let email = format!("{}@{}.com", local, domain);
        let err = validate_email_format(&email).unwrap_err();
        assert!(matches!(err, BridgeError::ValidationError(msg) if msg.contains("maximum length")));
    }
}

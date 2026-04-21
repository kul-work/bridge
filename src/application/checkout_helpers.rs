use sha2::{Digest, Sha256};

use crate::application::checkout_types::CheckoutRedirectUrls;
use crate::error::BridgeError;

/// Parse a decimal currency string (e.g. "9.99", "10") into integer cents.
/// Avoids f64 intermediary to prevent floating-point rounding errors.
pub(crate) fn parse_cents(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (negative, s) = if s.starts_with('-') {
        (true, &s[1..])
    } else {
        (false, s)
    };

    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() > 2 {
        return None;
    }

    let whole: i32 = parts[0].parse().ok()?;
    if whole < 0 {
        return None;
    }

    let cents = match parts.get(1) {
        Some(frac) => {
            let frac: String = frac.chars().take(2).collect();
            if frac.is_empty() {
                0
            } else if frac.len() == 1 {
                frac.parse::<i32>().ok()? * 10
            } else {
                frac.parse::<i32>().ok()?
            }
        }
        None => 0,
    };

    let result = whole * 100 + cents;
    Some(if negative { -result } else { result })
}

/// Format integer cents as a dollar string (e.g. 999 → "9.99", 1000 → "10.00")
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
        success_url: base_url.clone(),
        cancel_url: base_url,
    }
}

pub(crate) fn coinbase_amount_from_config(config: &serde_json::Value) -> Result<String, BridgeError> {
    if let Some(amount) = config.get("amount").and_then(|value| value.as_str()) {
        let normalized = amount.trim();
        if !normalized.is_empty() {
            return Ok(normalized.to_string());
        }
    }

    if let Some(amount_cents) = config.get("amount_cents").and_then(|value| value.as_i64()) {
        if amount_cents <= 0 {
            return Err(BridgeError::ConfigError(
                "Coinbase amount_cents must be positive".to_string(),
            ));
        }

        return Ok(format_cents_as_dollars(amount_cents as i32));
    }

    Err(BridgeError::ConfigError(
        "Missing Coinbase amount or amount_cents".to_string(),
    ))
}

pub(crate) fn extract_coinbase_checkout_url(data: &serde_json::Value) -> Option<&str> {
    data.pointer("/data/hosted_url")
        .and_then(|value| value.as_str())
        .or_else(|| data.pointer("/data/attributes/hosted_url").and_then(|value| value.as_str()))
        .or_else(|| data.pointer("/data/hostedUrl").and_then(|value| value.as_str()))
        .or_else(|| data.get("hosted_url").and_then(|value| value.as_str()))
        .or_else(|| data.get("url").and_then(|value| value.as_str()))
}

pub(crate) fn extract_coinbase_checkout_id(data: &serde_json::Value) -> Option<&str> {
    data.pointer("/data/id")
        .and_then(|value| value.as_str())
        .or_else(|| data.get("id").and_then(|value| value.as_str()))
}

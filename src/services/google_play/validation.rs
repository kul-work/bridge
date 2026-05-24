//! Token Validation Framework for Google Play Billing
//!
//! Provides different validation modes (Strict, Relaxed, Off) to support
//! varying levels of token validation in different environments:
//! - STRICT: Full validation (ERR tests, production)
//! - RELAXED: Basic validation only (staging)
//! - OFF: No validation (development, backward compatibility)
use crate::error::AppError;
use std::str::FromStr;

const MOCK_SUBSCRIPTION_TOKEN_PREFIX: &str = "mock-google-play-subscription:";

/// Token validation mode enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenValidationMode {
    #[default]
    Strict,  // Full validation: format, subscription match, expiration, API errors
    Relaxed, // Basic validation only: format, subscription match
    Off,     // No validation (current behavior)
}

impl FromStr for TokenValidationMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.eq_ignore_ascii_case("strict") {
            Ok(TokenValidationMode::Strict)
        } else if trimmed.eq_ignore_ascii_case("relaxed") {
            Ok(TokenValidationMode::Relaxed)
        } else if trimmed.eq_ignore_ascii_case("off") {
            Ok(TokenValidationMode::Off)
        } else {
            Err(format!(
                "Invalid token validation mode: '{}'. Use 'strict', 'relaxed', or 'off'",
                s
            ))
        }
    }
}

impl std::fmt::Display for TokenValidationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenValidationMode::Strict => write!(f, "strict"),
            TokenValidationMode::Relaxed => write!(f, "relaxed"),
            TokenValidationMode::Off => write!(f, "off"),
        }
    }
}

/// Token validator struct with validation methods
pub struct TokenValidator;

impl TokenValidator {
    /// Validate token format
    ///
    /// Rules:
    /// - Token must not be empty
    /// - Token must be at least 10 characters long
    /// - Token must not exceed 1000 characters
    /// - Token must not contain only whitespace
    /// - When mocking APIs (MOCK_EXTERNAL_APIS=true), reject obviously fake test tokens
    ///
    /// Note: Production (mock_external_apis=false) only enforces length/structure.
    /// Real token validation is done via Google Play API verification + user binding.
    pub fn validate_format(token: &str, mock_external_apis: bool) -> Result<(), AppError> {
        // Reject empty or whitespace-only tokens
        if token.trim().is_empty() {
            return Err(AppError::PaymentProviderError(
                "Purchase token cannot be empty".to_string(),
            ));
        }

        let len = token.len();

        // Reject tokens that are too short
        if len < 10 {
            return Err(AppError::PaymentProviderError(
                "Purchase token is too short (minimum 10 characters)".to_string(),
            ));
        }

        // Reject tokens that are too long (unrealistic for a purchase token)
        if len > 1000 {
            return Err(AppError::PaymentProviderError(
                "Purchase token is too long (maximum 1000 characters)".to_string(),
            ));
        }

        // Only reject obviously fake tokens when mocking APIs (dev/test).
        // In production with real Google Play APIs, we rely on API verification,
        // not heuristic patterns. Real tokens can have any characters.
        if mock_external_apis
            && (token.contains("invalid")
                || token.contains("not-a-valid")
                || token.contains("abc") && token.contains("!@#$")
                || (token.chars().all(|c| c.is_numeric())))
        {
            return Err(AppError::PaymentProviderError(
                "Invalid or revoked purchase token".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate subscription ID match
    ///
    /// Rules:
    /// - If token contains "mismatch", fail validation
    /// - Mock subscription tokens can encode their provider product id using
    ///   `mock-google-play-subscription:{subscription_id}:{unique_token}`.
    pub fn validate_subscription_match(
        token: &str,
        subscription_id: &str,
        _valid_subscriptions: &[String],
    ) -> Result<(), AppError> {
        // Token explicitly signals a mismatch
        if token.contains("mismatch") {
            return Err(AppError::PaymentProviderError(
                "Purchase token does not match the requested subscription ID".to_string(),
            ));
        }

        if let Some(token_subscription_id) = mock_subscription_id_from_token(token) {
            if token_subscription_id != subscription_id {
                return Err(AppError::PaymentProviderError(
                    "Purchase token does not match the requested subscription ID".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// **MOCK ONLY**: Validate token expiration
    ///
    /// This is a mock-only heuristic for testing. In production, token expiration
    /// is verified by Google Play API, not by string patterns.
    ///
    /// Rules (mock only):
    /// - Reject tokens containing "expired" keyword
    /// - This simulates Google Play's 60-day expiration rule for purchase tokens
    pub fn validate_expiration_mock(token: &str) -> Result<(), AppError> {
        if token.contains("expired") {
            return Err(AppError::PaymentProviderError(
                "Purchase token has expired (>60 days past subscription expiry)".to_string(),
            ));
        }

        Ok(())
    }

    /// **MOCK ONLY**: Validate API error responses
    ///
    /// This is a mock-only heuristic for testing. In production, API errors are
    /// returned directly by Google Play API, not inferred from token patterns.
    ///
    /// Rules (mock only):
    /// - Reject tokens containing "api-error" or "google-api-error" keywords
    /// - This simulates Google Play API being temporarily unavailable (5xx errors)
    pub fn validate_api_errors_mock(token: &str) -> Result<(), AppError> {
        if token.contains("api-error") || token.contains("google-api-error") {
            return Err(AppError::PaymentProviderError(
                "Google Play API temporarily unavailable (5xx error)".to_string(),
            ));
        }

        Ok(())
    }

    /// Apply validation based on mode
    ///
    /// Orchestrator method that applies different validation levels based on mode:
    /// - STRICT: Format check (length), subscription match, expiration, API errors
    /// - RELAXED: Format check and subscription match only
    /// - OFF: No validations (mocked APIs only)
    ///
    /// Format validation only checks structural constraints (length, non-empty).
    /// Token authenticity is verified by:
    /// - Real: Google Play API verification + user binding
    /// - Mocked: Mock response payloads that simulate Google Play responses
    pub fn apply_validation(
        mode: TokenValidationMode,
        token: &str,
        subscription_id: &str,
        mock_external_apis: bool,
    ) -> Result<(), AppError> {
        if mode == TokenValidationMode::Off {
            return Ok(());
        }

        // Shared checks for STRICT and RELAXED.
        Self::validate_format(token, mock_external_apis)?;
        Self::validate_subscription_match(token, subscription_id, &[])?;

        match mode {
            TokenValidationMode::Strict => {
                // Mock-only heuristics: only validate in test/mock environment.
                if mock_external_apis {
                    Self::validate_expiration_mock(token)?;
                    Self::validate_api_errors_mock(token)?;
                }
            }
            TokenValidationMode::Relaxed => {}
            TokenValidationMode::Off => {
                // Returned early above.
            }
        }

        Ok(())
    }
}

fn mock_subscription_id_from_token(token: &str) -> Option<&str> {
    token
        .strip_prefix(MOCK_SUBSCRIPTION_TOKEN_PREFIX)
        .and_then(|rest| rest.split(':').next())
        .filter(|subscription_id| !subscription_id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_mode_from_str() {
        assert_eq!("strict".parse::<TokenValidationMode>().unwrap(), TokenValidationMode::Strict);
        assert_eq!("relaxed".parse::<TokenValidationMode>().unwrap(), TokenValidationMode::Relaxed);
        assert_eq!("off".parse::<TokenValidationMode>().unwrap(), TokenValidationMode::Off);
        assert_eq!("STRICT".parse::<TokenValidationMode>().unwrap(), TokenValidationMode::Strict);
    }

    #[test]
    fn test_validation_mode_default() {
        assert_eq!(TokenValidationMode::default(), TokenValidationMode::Strict);
    }

    #[test]
    fn test_validate_format_empty() {
        assert!(TokenValidator::validate_format("", true).is_err());
        assert!(TokenValidator::validate_format("   ", true).is_err());
    }

    #[test]
    fn test_validate_format_too_short() {
        assert!(TokenValidator::validate_format("abc123", true).is_err());
    }

    #[test]
    fn test_validate_format_too_long() {
        assert!(TokenValidator::validate_format(&"a".repeat(1001), true).is_err());
    }

    #[test]
    fn test_validate_format_valid() {
        assert!(TokenValidator::validate_format("valid-token-abc123xyz789", true).is_ok());
        assert!(TokenValidator::validate_format("mock-google-play-subscription:premium_monthly:1234567890", true).is_ok());
        assert!(TokenValidator::validate_format("resubscribe-linking-required", true).is_ok());
    }

    #[test]
    fn test_validate_format_obviously_fake_when_mocking() {
        // When mock_external_apis=true, reject obviously fake patterns
        assert!(TokenValidator::validate_format("invalid-token-xyz", true).is_err());
        assert!(TokenValidator::validate_format("not-a-valid-purchase-token", true).is_err());
        assert!(TokenValidator::validate_format("12345", true).is_err()); // too short anyway
        assert!(TokenValidator::validate_format("1234567890", true).is_err()); // pure numeric
        assert!(TokenValidator::validate_format("abc!@#$%^&*()", true).is_err()); // obviously fake
    }

    #[test]
    fn test_validate_format_allows_keywords_in_prod() {
        // When mock_external_apis=false (production), allow tokens with "invalid" patterns
        // (real Google Play tokens can have any characters, validation happens via API)
        assert!(TokenValidator::validate_format("invalid-token-xyz-1234567890", false).is_ok());
        assert!(TokenValidator::validate_format("not-a-valid-purchase-token-1234567890", false).is_ok());
        // But still enforce length constraints
        assert!(TokenValidator::validate_format("12345", false).is_err()); // too short
        assert!(TokenValidator::validate_format(&"a".repeat(1001), false).is_err()); // too long
        // Pure numeric is OK if long enough
        assert!(TokenValidator::validate_format("1234567890", false).is_ok());
    }

    #[test]
    fn test_validate_subscription_match() {
        assert!(TokenValidator::validate_subscription_match("valid-token", "sub-123", &[]).is_ok());
        assert!(TokenValidator::validate_subscription_match("token-mismatch-sub", "sub-123", &[]).is_err());
        assert!(TokenValidator::validate_subscription_match("mock-google-play-subscription:premium_monthly:1234567890", "premium_monthly", &[]).is_ok());
        assert!(TokenValidator::validate_subscription_match("mock-google-play-subscription:premium_monthly:1234567890", "wrong_subscription_id", &[]).is_err());
    }

    #[test]
    fn test_validate_expiration_mock() {
        assert!(TokenValidator::validate_expiration_mock("valid-token").is_ok());
        assert!(TokenValidator::validate_expiration_mock("expired-token").is_err());
    }

    #[test]
    fn test_validate_api_errors_mock() {
        assert!(TokenValidator::validate_api_errors_mock("valid-token").is_ok());
        assert!(TokenValidator::validate_api_errors_mock("api-error-token").is_err());
        assert!(TokenValidator::validate_api_errors_mock("google-api-error-token").is_err());
    }

    #[test]
    fn test_apply_validation_off() {
        // OFF mode should pass anything
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Off,
            "",
            "sub-123",
            true
        )
        .is_ok());
    }

    #[test]
    fn test_apply_validation_relaxed() {
        // RELAXED should validate format and subscription match, but not expiration
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Relaxed,
            "valid-token-1234",
            "sub-123",
            true
        )
        .is_ok());

        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Relaxed,
            "",
            "sub-123",
            true
        )
        .is_err());

        // RELAXED doesn't care about expiration
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Relaxed,
            "expired-token-1234",
            "sub-123",
            true
        )
        .is_ok());
    }

    #[test]
    fn test_apply_validation_strict() {
        // STRICT should validate everything
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Strict,
            "valid-token-1234",
            "sub-123",
            true
        )
        .is_ok());

        // STRICT rejects empty
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Strict,
            "",
            "sub-123",
            true
        )
        .is_err());

        // STRICT rejects expiration
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Strict,
            "expired-token-1234",
            "sub-123",
            true
        )
        .is_err());

        // STRICT rejects API errors
        assert!(TokenValidator::apply_validation(
            TokenValidationMode::Strict,
            "api-error-token-1234",
            "sub-123",
            true
        )
        .is_err());
    }
}

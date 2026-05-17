const DIAGNOSTIC_REDACTION: &str = "[redacted]";
const DIAGNOSTIC_VISIBLE_SUFFIX_LEN: usize = 8;
const DIAGNOSTIC_HASH_LEN: usize = 12;

pub(crate) fn redact_with_prefix(value: &str) -> String {
    let suffix_chars: Vec<char> = value.chars().rev().take(DIAGNOSTIC_VISIBLE_SUFFIX_LEN).collect();
    let suffix: String = suffix_chars.into_iter().rev().collect();

    if suffix.is_empty() {
        DIAGNOSTIC_REDACTION.to_string()
    } else {
        format!("{}...{}", DIAGNOSTIC_REDACTION, suffix)
    }
}

pub(crate) fn diagnostic_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(value.as_bytes()))
        .chars()
        .take(DIAGNOSTIC_HASH_LEN)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_with_prefix_keeps_last_eight_chars() {
        assert_eq!(redact_with_prefix("1234567890abcdef"), "[redacted]...90abcdef");
        assert_eq!(redact_with_prefix("short"), "[redacted]...short");
        assert_eq!(redact_with_prefix(""), "[redacted]");
    }

    #[test]
    fn diagnostic_hash_is_short_and_stable() {
        let hash = diagnostic_hash("purchase-token");

        assert_eq!(hash.len(), 12);
        assert_eq!(hash, diagnostic_hash("purchase-token"));
    }
}

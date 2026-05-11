const DIAGNOSTIC_REDACTION: &str = "[redacted]";
const DIAGNOSTIC_VISIBLE_SUFFIX_LEN: usize = 8;

pub(crate) fn redact_with_prefix(value: &str) -> String {
    let suffix_chars: Vec<char> = value.chars().rev().take(DIAGNOSTIC_VISIBLE_SUFFIX_LEN).collect();
    let suffix: String = suffix_chars.into_iter().rev().collect();

    if suffix.is_empty() {
        DIAGNOSTIC_REDACTION.to_string()
    } else {
        format!("{}...{}", DIAGNOSTIC_REDACTION, suffix)
    }
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
}

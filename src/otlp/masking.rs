use once_cell::sync::Lazy;
use regex::Regex;

pub struct MaskingRules {
    pub global: Vec<Regex>,
}

static DEFAULT_MASKING_PATTERNS: &[&str] = &[
    ".*pass.*",
    ".*key.*",
    ".*secret.*",
    ".*credential.*",
    ".*passphrase.*",
];

static MASKING_RULES: Lazy<MaskingRules> = Lazy::new(|| {
    let global = DEFAULT_MASKING_PATTERNS
        .iter()
        .filter_map(|pattern| Regex::new(&format!("(?i){}", pattern)).ok())
        .collect();

    MaskingRules { global }
});

pub fn get_masking_rules() -> &'static MaskingRules {
    &MASKING_RULES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_masking_rules_returns_default_patterns() {
        let rules = get_masking_rules();
        assert_eq!(rules.global.len(), 5);
    }

    #[test]
    fn test_masking_rules_match_sensitive_keys() {
        let rules = get_masking_rules();

        let sensitive_keys = vec![
            "password",
            "api_key",
            "secret_token",
            "aws_credential",
            "passphrase",
            "PASSWORD",
            "API_KEY",
        ];

        for key in sensitive_keys {
            let matches = rules.global.iter().any(|r| r.is_match(key));
            assert!(matches, "Expected '{}' to match masking rules", key);
        }
    }

    #[test]
    fn test_masking_rules_do_not_match_safe_keys() {
        let rules = get_masking_rules();

        let safe_keys = vec!["username", "email", "timestamp", "request_id"];

        for key in safe_keys {
            let matches = rules.global.iter().any(|r| r.is_match(key));
            assert!(!matches, "Expected '{}' to NOT match masking rules", key);
        }
    }
}

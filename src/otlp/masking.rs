use once_cell::sync::OnceCell;
use regex::Regex;
use std::env;
use tracing::warn;

const DASH0_MASK_RULES_ENV: &str = "DASH0_MASK_RULES";

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

static MASKING_RULES: OnceCell<MaskingRules> = OnceCell::new();

fn patterns_to_rules(patterns: &[&str]) -> MaskingRules {
    let global = patterns
        .iter()
        .filter_map(|pattern| Regex::new(&format!("(?i){}", pattern)).ok())
        .collect();

    MaskingRules { global }
}

fn default_masking_rules() -> MaskingRules {
    patterns_to_rules(DEFAULT_MASKING_PATTERNS)
}

pub fn init_masking_rules() {
    MASKING_RULES.get_or_init(|| {
        match env::var(DASH0_MASK_RULES_ENV) {
            Ok(value) => {
                match serde_json::from_str::<Vec<String>>(&value) {
                    Ok(patterns) => {
                        let pattern_refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
                        patterns_to_rules(&pattern_refs)
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse {} as JSON array of strings: {}. Using default masking rules.",
                            DASH0_MASK_RULES_ENV, e
                        );
                        default_masking_rules()
                    }
                }
            }
            Err(_) => default_masking_rules(),
        }
    });
}

pub fn get_masking_rules() -> &'static MaskingRules {
    MASKING_RULES.get_or_init(default_masking_rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_masking_rules_has_five_patterns() {
        let rules = default_masking_rules();
        assert_eq!(rules.global.len(), 5);
    }

    #[test]
    fn test_default_rules_match_sensitive_keys() {
        let rules = default_masking_rules();

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
    fn test_default_rules_do_not_match_safe_keys() {
        let rules = default_masking_rules();

        let safe_keys = vec!["username", "email", "timestamp", "request_id"];

        for key in safe_keys {
            let matches = rules.global.iter().any(|r| r.is_match(key));
            assert!(!matches, "Expected '{}' to NOT match masking rules", key);
        }
    }

    #[test]
    fn test_patterns_to_rules_creates_case_insensitive_regex() {
        let patterns = &[".*token.*"];
        let rules = patterns_to_rules(patterns);

        assert!(rules.global[0].is_match("token"));
        assert!(rules.global[0].is_match("TOKEN"));
        assert!(rules.global[0].is_match("my_token_here"));
    }

    #[test]
    fn test_patterns_to_rules_skips_invalid_regex() {
        let patterns = &[".*valid.*", "[invalid"];
        let rules = patterns_to_rules(patterns);

        assert_eq!(rules.global.len(), 1);
    }

    #[test]
    fn test_custom_patterns_from_json() {
        let json = r#"[".*custom.*", ".*private.*"]"#;
        let patterns: Vec<String> = serde_json::from_str(json).unwrap();
        let pattern_refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
        let rules = patterns_to_rules(&pattern_refs);

        assert_eq!(rules.global.len(), 2);
        assert!(rules.global.iter().any(|r| r.is_match("custom_field")));
        assert!(rules.global.iter().any(|r| r.is_match("private_data")));
    }
}

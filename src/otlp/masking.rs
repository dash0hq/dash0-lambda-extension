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

const MASKED_VALUE: &str = "******";

fn should_mask(key: &str, rules: &MaskingRules) -> bool {
    rules.global.iter().any(|r| r.is_match(key))
}

fn mask_value(value: &mut serde_json::Value, rules: &MaskingRules) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if should_mask(key, rules) {
                    *val = serde_json::Value::String(MASKED_VALUE.to_string());
                } else {
                    mask_value(val, rules);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                mask_value(item, rules);
            }
        }
        _ => {}
    }
}

pub fn mask_json_string(json_str: &str) -> String {
    let rules = get_masking_rules();

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(mut value) => {
            mask_value(&mut value, rules);
            serde_json::to_string(&value).unwrap_or_else(|_| json_str.to_string())
        }
        Err(_) => json_str.to_string(),
    }
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

    #[test]
    fn test_mask_json_string_masks_sensitive_keys() {
        let input = r#"{"username": "john", "password": "secret123"}"#;
        let result = mask_json_string(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["username"], "john");
        assert_eq!(parsed["password"], "******");
    }

    #[test]
    fn test_mask_json_string_masks_nested_objects() {
        let input = r#"{"user": {"name": "john", "api_key": "abc123"}}"#;
        let result = mask_json_string(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["user"]["name"], "john");
        assert_eq!(parsed["user"]["api_key"], "******");
    }

    #[test]
    fn test_mask_json_string_masks_entire_array_when_key_matches() {
        let input = r#"{"secrets": ["a", "b", "c"], "names": ["john", "jane"]}"#;
        let result = mask_json_string(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["secrets"], "******");
        assert!(parsed["names"].is_array());
    }

    #[test]
    fn test_mask_json_string_recurses_into_arrays() {
        let input = r#"{"items": [{"name": "item1", "secret_code": "xyz"}]}"#;
        let result = mask_json_string(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["items"][0]["name"], "item1");
        assert_eq!(parsed["items"][0]["secret_code"], "******");
    }

    #[test]
    fn test_mask_json_string_returns_original_on_invalid_json() {
        let input = "not valid json";
        let result = mask_json_string(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_mask_json_string_handles_deeply_nested() {
        let input = r#"{"level1": {"level2": {"level3": {"password": "deep"}}}}"#;
        let result = mask_json_string(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["level1"]["level2"]["level3"]["password"], "******");
    }
}

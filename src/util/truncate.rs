use crate::config::max_event_payload_size;
use crate::otlp::masking::mask_json_string;

fn truncate_plain(masked_payload: &str, max_size: usize) -> String {
    let mut truncate_at = max_size;
    while truncate_at > 0 && !masked_payload.is_char_boundary(truncate_at) {
        truncate_at -= 1;
    }
    tracing::trace!(
        "[{}] Truncating payload from {} to {} bytes.",
        crate::log_prefix(),
        masked_payload.len(),
        truncate_at
    );
    masked_payload[..truncate_at].to_string()
}

const TRUNCATED_MARKER: &str = "[truncated]";

fn truncate_json_object(masked_payload: &str, max_size: usize) -> String {
    let mut obj: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str(masked_payload) {
            Ok(obj) => obj,
            Err(_) => return truncate_plain(masked_payload, max_size),
        };

    let keys: Vec<String> = obj.keys().cloned().collect();

    // Iterate from the last field backwards, truncating until we fit
    for key in keys.iter().rev() {
        if let Some(val) = obj.get(key) {
            if val.as_str() == Some(TRUNCATED_MARKER) {
                continue;
            }
        }
        obj.insert(
            key.clone(),
            serde_json::Value::String(TRUNCATED_MARKER.to_string()),
        );

        let serialized = serde_json::to_string(&obj).unwrap_or_default();
        if serialized.len() <= max_size {
            tracing::trace!(
                "[{}] Truncated JSON field '{}' to fit within {} bytes.",
                crate::log_prefix(),
                key,
                max_size
            );
            return serialized;
        }
    }

    // All fields truncated but still too large — fall back to plain truncation
    let serialized = serde_json::to_string(&obj).unwrap_or_default();
    if serialized.len() <= max_size {
        return serialized;
    }
    truncate_plain(&serialized, max_size)
}

pub fn process_payload(payload_bytes: &[u8]) -> String {
    let payload_str = String::from_utf8_lossy(payload_bytes);
    let masked_payload = mask_json_string(&payload_str);

    let max_size = max_event_payload_size();
    if masked_payload.len() <= max_size {
        return masked_payload;
    }

    truncate_json_object(&masked_payload, max_size)
}

#[cfg(test)]
mod tests {
    use super::process_payload;
    use serial_test::serial;

    #[test]
    #[serial]
    fn process_payload_truncates_at_char_boundary_for_non_json() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // Non-JSON with multi-byte UTF-8 characters
        let base = "a".repeat(1020);
        let payload = format!("{}생생생생", base); // 1020 + 12 = 1032 bytes

        let result = process_payload(payload.as_bytes());

        assert!(result.len() <= 1024);
        assert!(result.chars().count() > 0);
        assert!(result.is_char_boundary(result.len()));

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_no_truncation_when_under_limit() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // Use a non-JSON payload to avoid masking interference
        let payload = "short payload";
        let result = process_payload(payload.as_bytes());

        assert_eq!(result, payload);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_truncates_json_last_field_first() {
        // DASH0_MAX_EVENT_PAYLOAD is in KB, "1" = 1024 bytes
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // {"a":"small","b":"<2000 x's>"} is well over 1024 bytes
        let big_value = "x".repeat(2000);
        let payload = format!(r#"{{"a":"small","b":"{}"}}"#, big_value);

        let result = process_payload(payload.as_bytes());

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], "small");
        assert_eq!(parsed["b"], "[truncated]");
        assert!(result.len() <= 1024);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_truncates_multiple_json_fields_from_end() {
        // "1" = 1024 bytes — both b and c need truncation
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        let payload = format!(
            r#"{{"a":"ok","b":"{}","c":"{}"}}"#,
            "y".repeat(1000),
            "z".repeat(1000)
        );

        let result = process_payload(payload.as_bytes());

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], "ok");
        assert_eq!(parsed["b"], "[truncated]");
        assert_eq!(parsed["c"], "[truncated]");
        assert!(result.len() <= 1024);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_json_array_falls_back_to_plain_truncation() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        let payload = format!(r#"["{}"]"#, "a".repeat(2000));

        let result = process_payload(payload.as_bytes());

        assert!(result.len() <= 1024);
        // Not valid JSON anymore since it was plain-truncated
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_err());

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }
}

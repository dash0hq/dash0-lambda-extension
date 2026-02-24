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

/// Find all JSON string literals and return their content byte ranges (start..end between quotes).
fn find_json_strings(json: &[u8]) -> Vec<(usize, usize)> {
    let mut strings = Vec::new();
    let mut i = 0;
    while i < json.len() {
        if json[i] == b'"' {
            let content_start = i + 1;
            i += 1;
            while i < json.len() {
                if json[i] == b'\\' {
                    i += 2;
                } else if json[i] == b'"' {
                    strings.push((content_start, i));
                    break;
                } else {
                    i += 1;
                }
            }
        }
        i += 1;
    }
    strings
}

/// Truncate long JSON string values directly in the raw text — no parsing needed.
/// Scans for all string literals once, then replaces them with "[truncated]"
/// from longest to shortest until the result fits within max_size.
fn truncate_json_strings(json: &str, max_size: usize) -> String {
    let mut strings = find_json_strings(json.as_bytes());
    // Sort longest first
    strings.sort_by(|(s1, e1), (s2, e2)| (e2 - s2).cmp(&(e1 - s1)));

    let marker = TRUNCATED_MARKER.as_bytes();
    let mut result = json.as_bytes().to_vec();
    let mut current_len = result.len();

    for i in 0..strings.len() {
        if current_len <= max_size {
            break;
        }

        let (content_start, content_end) = strings[i];
        let content_len = content_end - content_start;
        if content_len <= marker.len() {
            continue;
        }

        // Splice: replace content_start..content_end with the marker
        let saved = content_len - marker.len();
        result.splice(content_start..content_end, marker.iter().copied());
        current_len -= saved;

        // All positions after the splice point shift left by `saved` bytes
        for j in 0..strings.len() {
            if j != i && strings[j].0 > content_start {
                strings[j].0 -= saved;
                strings[j].1 -= saved;
            }
        }
    }

    // If still too large after all string truncations, fall back to plain truncation
    let result_str = String::from_utf8_lossy(&result).into_owned();
    if result_str.len() <= max_size {
        result_str
    } else {
        truncate_plain(&result_str, max_size)
    }
}

pub fn process_payload(payload_bytes: &[u8]) -> String {
    let payload_str = String::from_utf8_lossy(payload_bytes);
    let masked_payload = mask_json_string(&payload_str);

    let max_size = max_event_payload_size();
    if masked_payload.len() <= max_size {
        return masked_payload;
    }

    std::panic::catch_unwind(|| truncate_json_strings(&masked_payload, max_size))
        .unwrap_or_else(|_| truncate_plain(&masked_payload, max_size))
}

#[cfg(test)]
mod tests {
    use super::process_payload;
    use serial_test::serial;

    #[test]
    #[serial]
    fn process_payload_truncates_at_char_boundary_for_non_json() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

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

        let payload = "short payload";
        let result = process_payload(payload.as_bytes());

        assert_eq!(result, payload);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_truncates_longest_value_first() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // "b" is the longest string — it gets replaced with [truncated]
        let payload = format!(r#"{{"a":"small","b":"{}"}}"#, "x".repeat(2000));

        let result = process_payload(payload.as_bytes());

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], "small");
        assert_eq!(parsed["b"], "[truncated]");
        assert!(result.len() <= 1024);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_truncates_multiple_values_until_fits() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // Both "b" and "c" are long — both need to be truncated to fit
        let payload = format!(
            r#"{{"a":"ok","b":"{}","c":"{}"}}"#,
            "y".repeat(1000),
            "z".repeat(1000)
        );

        let result = process_payload(payload.as_bytes());

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], "ok");
        assert_eq!(parsed["b"], "[truncated]");
        assert_eq!(parsed["c"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_json_array_preserves_valid_json() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        let payload = format!(r#"["{}"]"#, "a".repeat(2000));

        let result = process_payload(payload.as_bytes());

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[0], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_handles_nested_json() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        let payload = format!(
            r#"{{"meta":{{"status":"ok"}},"data":{{"nested":{{"deep":"{}"}}}}}}"#,
            "x".repeat(2000)
        );

        let result = process_payload(payload.as_bytes());

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["meta"]["status"], "ok");
        assert_eq!(parsed["data"]["nested"]["deep"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_only_truncates_enough_to_fit() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // "c" (900) is longest, truncating it saves enough — "b" (200) is left alone
        let payload = format!(
            r#"{{"a":"ok","b":"{}","c":"{}"}}"#,
            "y".repeat(200),
            "z".repeat(900)
        );

        let result = process_payload(payload.as_bytes());

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], "ok");
        assert_eq!(parsed["b"], "y".repeat(200));
        assert_eq!(parsed["c"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }
}

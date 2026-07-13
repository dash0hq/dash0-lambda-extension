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

/// A string literal directly followed by ':' (ignoring whitespace) is an
/// object key — replacing those would destroy field names and collapse
/// distinct keys into duplicate "[truncated]" entries.
fn is_object_key(json: &[u8], content_end: usize) -> bool {
    let mut i = content_end + 1; // skip the closing quote
    while i < json.len() && json[i].is_ascii_whitespace() {
        i += 1;
    }
    i < json.len() && json[i] == b':'
}

/// Truncate long JSON string values directly in the raw text — no parsing needed.
/// Picks the longest values first until the projected size fits within
/// max_size, then rebuilds the payload in a single pass with each picked
/// value replaced by "[truncated]". Object keys are never replaced.
fn truncate_json_strings(json: &str, max_size: usize) -> String {
    let bytes = json.as_bytes();
    let marker_len = TRUNCATED_MARKER.len();
    let mut candidates: Vec<(usize, usize)> = find_json_strings(bytes)
        .into_iter()
        .filter(|&(start, end)| end - start > marker_len && !is_object_key(bytes, end))
        .collect();

    // If replacing every candidate still can't fit, skip straight to plain
    // truncation instead of doing all that replacement work for nothing.
    let max_savings: usize = candidates
        .iter()
        .map(|&(start, end)| (end - start) - marker_len)
        .sum();
    if json.len() - max_savings > max_size {
        return truncate_plain(json, max_size);
    }

    // Longest first, pick until the projected size fits
    candidates.sort_by_key(|&(start, end)| std::cmp::Reverse(end - start));
    let mut projected = json.len();
    let mut picked = Vec::new();
    for &(start, end) in &candidates {
        if projected <= max_size {
            break;
        }
        projected -= (end - start) - marker_len;
        picked.push((start, end));
    }

    // Rebuild in one pass, in document order
    picked.sort_unstable();
    let mut result = Vec::with_capacity(projected);
    let mut pos = 0;
    for &(start, end) in &picked {
        result.extend_from_slice(&bytes[pos..start]);
        result.extend_from_slice(TRUNCATED_MARKER.as_bytes());
        pos = end;
    }
    result.extend_from_slice(&bytes[pos..]);

    // Replaced ranges begin and end at ASCII quotes, so the buffer is still
    // valid UTF-8
    let result_str =
        String::from_utf8(result).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    if result_str.len() <= max_size {
        result_str
    } else {
        truncate_plain(&result_str, max_size)
    }
}

pub fn process_payload(payload_str: &str) -> String {
    let masked_payload = mask_json_string(payload_str);

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

        let result = process_payload(&payload);

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
        let result = process_payload(&payload);

        assert_eq!(result, payload);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_truncates_longest_value_first() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // "b" is the longest string — it gets replaced with [truncated]
        let payload = format!(r#"{{"a":"small","b":"{}"}}"#, "x".repeat(2000));

        let result = process_payload(&payload);

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

        let result = process_payload(&payload);

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

        let result = process_payload(&payload);

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

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["meta"]["status"], "ok");
        assert_eq!(parsed["data"]["nested"]["deep"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_masks_before_truncation() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // The huge value sits under a masked key — masking shrinks it to "****"
        // before the size check, so nothing gets truncated
        let payload = format!(r#"{{"password":"{}","data":"ok"}}"#, "s".repeat(2000));

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["password"], "****");
        assert_eq!(parsed["data"], "ok");
        assert!(!result.contains("[truncated]"));

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_masks_and_truncates_when_still_over_limit() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        let payload = format!(
            r#"{{"api_token":"{}","data":"{}"}}"#,
            "t".repeat(500),
            "d".repeat(2000)
        );

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["api_token"], "****");
        assert_eq!(parsed["data"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_handles_escaped_quotes() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        let payload = format!(
            r#"{{"quoted":"he said \"hi\" to me","big":"{}"}}"#,
            "x".repeat(2000)
        );

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["quoted"], "he said \"hi\" to me");
        assert_eq!(parsed["big"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_falls_back_to_plain_truncation_without_long_strings() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // No string values to shrink — a big array of numbers can only be
        // plain-truncated
        let payload = format!("[{}1]", "1,".repeat(2000));

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_never_truncates_object_keys() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // Both keys are longer than the marker; only values may be replaced
        let long_key_a = "a".repeat(100);
        let long_key_b = "b".repeat(100);
        let payload = format!(
            r#"{{"{}":"{}","{}":"{}"}}"#,
            long_key_a,
            "x".repeat(1000),
            long_key_b,
            "y".repeat(1000)
        );

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[&long_key_a], "[truncated]");
        assert_eq!(parsed[&long_key_b], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn process_payload_handles_many_small_strings_quickly() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "1");

        // 50k strings of 30 chars (~1.6MB): replacing all of them can't reach
        // the limit, so the feasibility pre-check must skip straight to plain
        // truncation. Before that check this case took >10 seconds.
        let item = format!("\"{}\"", "x".repeat(30));
        let payload = format!("[{}]", vec![item; 50_000].join(","));

        let start = std::time::Instant::now();
        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "truncation took {:?}",
            start.elapsed()
        );

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

        let result = process_payload(&payload);

        assert!(result.len() <= 1024);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], "ok");
        assert_eq!(parsed["b"], "y".repeat(200));
        assert_eq!(parsed["c"], "[truncated]");

        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }
}

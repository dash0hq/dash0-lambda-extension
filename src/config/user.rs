/// Return true if DASH0_SEND_ON_INVOCATION_END is set to a truthy value.
/// Defaults to true when unset or unrecognized.
pub fn is_send_on_invocation_end() -> bool {
    match std::env::var("DASH0_SEND_ON_INVOCATION_END") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => true,
    }
}

pub fn is_auto_instrumented_disabled() -> bool {
    if std::env::var("AWS_LAMBDA_EXEC_WRAPPER").is_err() {
        return true;
    }

    if is_telemetry_traces_disabled() {
        return true;
    }

    match std::env::var("DASH0_DISABLE_AUTO_INSTRUMENTATION") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

/// Maximum size, in KB, of a captured Lambda event or return value payload
/// before the extension truncates it. Defaults to 1MB.
const DEFAULT_MAX_EVENT_PAYLOAD_KB: usize = 1024;

pub fn max_event_payload_size() -> usize {
    match std::env::var("DASH0_MAX_EVENT_PAYLOAD") {
        Ok(val) => val.parse::<usize>().unwrap_or(DEFAULT_MAX_EVENT_PAYLOAD_KB) * 1024,
        Err(_) => DEFAULT_MAX_EVENT_PAYLOAD_KB * 1024,
    }
}

pub fn request_timeout_ms() -> u64 {
    match std::env::var("DASH0_REQUEST_TIMEOUT") {
        Ok(val) => val.parse::<u64>().unwrap_or(2000),
        Err(_) => 2000,
    }
}

/// Buffered telemetry (all signals) that triggers an early flush while an
/// invocation is still running. DASH0_FLUSH_THRESHOLD in KB, default 2048 (2 MiB).
pub fn flush_threshold_bytes() -> usize {
    std::env::var("DASH0_FLUSH_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2048)
        * 1024
}

/// Longest time buffered telemetry waits while an invocation is still running.
/// DASH0_FLUSH_INTERVAL in ms, default 10000.
pub fn flush_interval_ms() -> u64 {
    std::env::var("DASH0_FLUSH_INTERVAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10_000)
}

/// Hard limit for buffered telemetry (all signals). Incoming telemetry above it
/// is dropped. DASH0_MAX_BUFFER_SIZE in KB; defaults to 10% of the function
/// memory, clamped to 8–64 MiB.
pub fn max_buffer_bytes() -> usize {
    if let Some(kb) = std::env::var("DASH0_MAX_BUFFER_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return kb * 1024;
    }
    let function_mb = std::env::var("AWS_LAMBDA_FUNCTION_MEMORY_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1024);
    (function_mb * 1024 * 1024 / 10).clamp(8 * 1024 * 1024, 64 * 1024 * 1024)
}

pub fn request_retries() -> usize {
    match std::env::var("DASH0_REQUEST_RETRIES") {
        Ok(val) => val.parse::<usize>().unwrap_or(1),
        Err(_) => 1,
    }
}

/// Whether OTLP request bodies should be gzip-compressed before export.
/// Controlled by DASH0_COMPRESSION, accepting `gzip` or `none`; defaults to `gzip`.
///
/// Intentionally uses a DASH0_-prefixed variable rather than
/// OTEL_EXPORTER_OTLP_COMPRESSION: Lambda env vars are shared with the function
/// runtime, so the in-function SDK would read the same variable and start
/// compressing its exports to the extension, which our receiver does not decode.
pub fn is_compression_enabled() -> bool {
    match std::env::var("DASH0_COMPRESSION") {
        Ok(val) => !val.eq_ignore_ascii_case("none"),
        Err(_) => true,
    }
}

pub fn is_create_payload_log_records() -> bool {
    match std::env::var("DASH0_CREATE_PAYLOAD_LOG_RECORDS") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => true,
    }
}

pub fn get_dash0_dataset() -> Option<String> {
    match std::env::var("DASH0_DATASET") {
        Ok(val) if !val.is_empty() => Some(val),
        _ => None,
    }
}

const DEFAULT_LOG_LEVEL: &str = "warn";

pub fn extension_log_level() -> String {
    std::env::var("DASH0_EXTENSION_LOG_LEVEL").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string())
}

pub fn is_xray_traces_enabled() -> bool {
    match std::env::var("DASH0_XRAY_TRACES_ENABLED") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

pub fn is_telemetry_log_collection_disabled() -> bool {
    match std::env::var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

pub fn is_telemetry_metrics_disabled() -> bool {
    match std::env::var("DASH0_DISABLE_TELEMETRY_METRICS") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

/// When true, the handler span for an API Gateway- or ALB-triggered
/// invocation is renamed to `<method> <route>` (just `<method>` for ALB, which
/// reports no route). Off by default so existing span names don't change under
/// users. The env var keeps its API Gateway name for backwards compatibility.
/// See `http_attributes::extract_span_name`.
pub fn is_api_gateway_span_name_enabled() -> bool {
    match std::env::var("DASH0_ENABLE_API_GATEWAY_SPAN_NAME") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

/// Comma-separated allow-list of request header names to capture as
/// `http.request.header.*` for API Gateway-triggered invocations. Empty by
/// default: headers may carry PII, auth tokens, or cookies.
pub fn api_gateway_request_headers_to_capture() -> String {
    std::env::var("DASH0_API_GATEWAY_REQUEST_HEADERS_TO_CAPTURE").unwrap_or_default()
}

/// Comma-separated allow-list of response header names to capture as
/// `http.response.header.*` for API Gateway-triggered invocations.
pub fn api_gateway_response_headers_to_capture() -> String {
    std::env::var("DASH0_API_GATEWAY_RESPONSE_HEADERS_TO_CAPTURE").unwrap_or_default()
}

/// When true, captures the request query string as `url.query` for API
/// Gateway-triggered invocations. Off by default: query strings can carry
/// signed-URL tokens or other sensitive values.
pub fn is_api_gateway_query_string_capture_enabled() -> bool {
    match std::env::var("DASH0_CAPTURE_API_GATEWAY_QUERY_STRING") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

pub fn is_telemetry_traces_disabled() -> bool {
    match std::env::var("DASH0_DISABLE_TELEMETRY_TRACES") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_auto_instrumented_disabled, is_compression_enabled, is_send_on_invocation_end,
        is_telemetry_log_collection_disabled, is_telemetry_metrics_disabled,
        is_telemetry_traces_disabled, max_event_payload_size,
    };
    use serial_test::serial;

    #[test]
    #[serial]
    fn compression_enabled_by_default() {
        std::env::remove_var("DASH0_COMPRESSION");
        assert!(is_compression_enabled());
    }

    #[test]
    #[serial]
    fn compression_enabled_for_gzip_value() {
        for val in ["gzip", "GZIP", "Gzip", "anything-else"] {
            std::env::set_var("DASH0_COMPRESSION", val);
            assert!(is_compression_enabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_COMPRESSION");
    }

    #[test]
    #[serial]
    fn compression_disabled_for_none_value() {
        for val in ["none", "NONE", "None"] {
            std::env::set_var("DASH0_COMPRESSION", val);
            assert!(!is_compression_enabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_COMPRESSION");
    }

    #[test]
    #[serial]
    fn defaults_to_true_when_missing() {
        std::env::remove_var("DASH0_SEND_ON_INVOCATION_END");
        assert!(is_send_on_invocation_end());
    }

    #[test]
    #[serial]
    fn recognizes_truthy_values() {
        for val in ["1", "true", "TRUE", "True", "yes", "YES", "Yes", "y", "Y"] {
            std::env::set_var("DASH0_SEND_ON_INVOCATION_END", val);
            assert!(is_send_on_invocation_end(), "value {}", val);
        }
        std::env::remove_var("DASH0_SEND_ON_INVOCATION_END");
    }

    #[test]
    #[serial]
    fn false_for_other_values() {
        for val in ["0", "false", "no", "maybe", ""] {
            std::env::set_var("DASH0_SEND_ON_INVOCATION_END", val);
            assert!(!is_send_on_invocation_end(), "value {}", val);
        }
        std::env::remove_var("DASH0_SEND_ON_INVOCATION_END");
    }

    #[test]
    #[serial]
    fn auto_instrumentation_disabled_when_wrapper_missing() {
        std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
        assert!(is_auto_instrumented_disabled());
    }

    #[test]
    #[serial]
    fn auto_instrumentation_enabled_when_wrapper_present() {
        std::env::set_var("AWS_LAMBDA_EXEC_WRAPPER", "/opt/lumigo_wrapper");
        std::env::remove_var("DASH0_DISABLE_AUTO_INSTRUMENTATION");
        assert!(!is_auto_instrumented_disabled());
        std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
    }

    #[test]
    #[serial]
    fn auto_instrumentation_disabled_explicitly() {
        std::env::set_var("AWS_LAMBDA_EXEC_WRAPPER", "/opt/lumigo_wrapper");
        std::env::set_var("DASH0_DISABLE_AUTO_INSTRUMENTATION", "true");
        assert!(is_auto_instrumented_disabled());
        std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
        std::env::remove_var("DASH0_DISABLE_AUTO_INSTRUMENTATION");
    }

    #[test]
    #[serial]
    fn flush_settings_parse_kb_and_ms_with_defaults() {
        use super::{flush_interval_ms, flush_threshold_bytes};
        std::env::remove_var("DASH0_FLUSH_THRESHOLD");
        std::env::remove_var("DASH0_FLUSH_INTERVAL");
        assert_eq!(flush_threshold_bytes(), 2048 * 1024);
        assert_eq!(flush_interval_ms(), 10_000);

        std::env::set_var("DASH0_FLUSH_THRESHOLD", "300");
        std::env::set_var("DASH0_FLUSH_INTERVAL", "0");
        assert_eq!(flush_threshold_bytes(), 300 * 1024);
        assert_eq!(flush_interval_ms(), 0);

        std::env::set_var("DASH0_FLUSH_THRESHOLD", "2MB");
        std::env::set_var("DASH0_FLUSH_INTERVAL", "-1");
        assert_eq!(flush_threshold_bytes(), 2048 * 1024);
        assert_eq!(flush_interval_ms(), 10_000);
        std::env::remove_var("DASH0_FLUSH_THRESHOLD");
        std::env::remove_var("DASH0_FLUSH_INTERVAL");
    }

    #[test]
    #[serial]
    fn max_buffer_is_ten_percent_of_function_memory_clamped() {
        use super::max_buffer_bytes;
        const MIB: usize = 1024 * 1024;
        std::env::remove_var("DASH0_MAX_BUFFER_SIZE");
        for (memory_mb, expected) in [
            ("128", 128 * MIB / 10), // 12.8 MiB, inside the bounds
            ("2048", 64 * MIB),      // 204.8 MiB, clamped to 64 MiB
            ("64", 8 * MIB),         // 6.4 MiB, clamped to 8 MiB
        ] {
            std::env::set_var("AWS_LAMBDA_FUNCTION_MEMORY_SIZE", memory_mb);
            assert_eq!(max_buffer_bytes(), expected, "{} MB function", memory_mb);
        }
        std::env::set_var("DASH0_MAX_BUFFER_SIZE", "100");
        assert_eq!(max_buffer_bytes(), 100 * 1024, "explicit KB value wins");
        std::env::remove_var("DASH0_MAX_BUFFER_SIZE");
        std::env::remove_var("AWS_LAMBDA_FUNCTION_MEMORY_SIZE");
    }

    #[test]
    #[serial]
    fn max_event_payload_size_defaults_to_1mb() {
        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
        assert_eq!(max_event_payload_size(), 1024 * 1024);
    }

    #[test]
    #[serial]
    fn max_event_payload_size_parses_value() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "100");
        assert_eq!(max_event_payload_size(), 100 * 1024);
        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn max_event_payload_size_handles_invalid_value() {
        std::env::set_var("DASH0_MAX_EVENT_PAYLOAD", "not_a_number");
        assert_eq!(max_event_payload_size(), 1024 * 1024);
        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
    }

    #[test]
    #[serial]
    fn telemetry_log_collection_enabled_by_default() {
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION");
        assert!(!is_telemetry_log_collection_disabled());
    }

    #[test]
    #[serial]
    fn telemetry_log_collection_disabled_with_truthy_values() {
        for val in ["1", "true", "TRUE", "True", "yes", "YES", "Yes", "y", "Y"] {
            std::env::set_var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION", val);
            assert!(is_telemetry_log_collection_disabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION");
    }

    #[test]
    #[serial]
    fn telemetry_log_collection_not_disabled_with_falsy_values() {
        for val in ["0", "false", "no", "maybe", ""] {
            std::env::set_var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION", val);
            assert!(!is_telemetry_log_collection_disabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION");
    }

    #[test]
    #[serial]
    fn telemetry_metrics_enabled_by_default() {
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_METRICS");
        assert!(!is_telemetry_metrics_disabled());
    }

    #[test]
    #[serial]
    fn telemetry_metrics_disabled_with_truthy_values() {
        for val in ["1", "true", "TRUE", "True", "yes", "YES", "Yes", "y", "Y"] {
            std::env::set_var("DASH0_DISABLE_TELEMETRY_METRICS", val);
            assert!(is_telemetry_metrics_disabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_METRICS");
    }

    #[test]
    #[serial]
    fn telemetry_metrics_not_disabled_with_falsy_values() {
        for val in ["0", "false", "no", "maybe", ""] {
            std::env::set_var("DASH0_DISABLE_TELEMETRY_METRICS", val);
            assert!(!is_telemetry_metrics_disabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_METRICS");
    }

    #[test]
    #[serial]
    fn telemetry_traces_enabled_by_default() {
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_TRACES");
        assert!(!is_telemetry_traces_disabled());
    }

    #[test]
    #[serial]
    fn telemetry_traces_disabled_with_truthy_values() {
        for val in ["1", "true", "TRUE", "True", "yes", "YES", "Yes", "y", "Y"] {
            std::env::set_var("DASH0_DISABLE_TELEMETRY_TRACES", val);
            assert!(is_telemetry_traces_disabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_TRACES");
    }

    #[test]
    #[serial]
    fn telemetry_traces_not_disabled_with_falsy_values() {
        for val in ["0", "false", "no", "maybe", ""] {
            std::env::set_var("DASH0_DISABLE_TELEMETRY_TRACES", val);
            assert!(!is_telemetry_traces_disabled(), "value {}", val);
        }
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_TRACES");
    }

    #[test]
    #[serial]
    fn telemetry_traces_disabled_implies_auto_instrumentation_disabled() {
        std::env::set_var("AWS_LAMBDA_EXEC_WRAPPER", "/opt/lumigo_wrapper");
        std::env::remove_var("DASH0_DISABLE_AUTO_INSTRUMENTATION");
        std::env::set_var("DASH0_DISABLE_TELEMETRY_TRACES", "true");
        assert!(is_auto_instrumented_disabled());
        std::env::remove_var("DASH0_DISABLE_TELEMETRY_TRACES");
        std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
    }
}

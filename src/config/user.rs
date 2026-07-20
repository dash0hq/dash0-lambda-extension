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

pub fn max_event_payload_size() -> usize {
    match std::env::var("DASH0_MAX_EVENT_PAYLOAD") {
        Ok(val) => val.parse::<usize>().unwrap_or(4) * 1024,
        Err(_) => 4 * 1024,
    }
}

pub fn request_timeout_ms() -> u64 {
    match std::env::var("DASH0_REQUEST_TIMEOUT") {
        Ok(val) => val.parse::<u64>().unwrap_or(2000),
        Err(_) => 2000,
    }
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
    fn max_event_payload_size_defaults_to_4kb() {
        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
        assert_eq!(max_event_payload_size(), 4 * 1024);
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
        assert_eq!(max_event_payload_size(), 4 * 1024);
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

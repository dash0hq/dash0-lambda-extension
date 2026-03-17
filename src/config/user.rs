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
        Ok(val) => val.parse::<usize>().unwrap_or(20) * 1024,
        Err(_) => 20 * 1024,
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

pub fn is_extract_span_links_in_consumer() -> bool {
    match std::env::var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => true,
    }
}

pub fn is_remove_lambda_parent_span() -> bool {
    match std::env::var("DASH0_REMOVE_LAMBDA_PARENT_SPAN") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
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

pub fn is_telemetry_log_collection_disabled() -> bool {
    match std::env::var("DASH0_DISABLE_TELEMETRY_LOG_COLLECTION") {
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
        is_auto_instrumented_disabled, is_send_on_invocation_end,
        is_telemetry_log_collection_disabled, max_event_payload_size,
    };
    use serial_test::serial;

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
    fn max_event_payload_size_defaults_to_20kb() {
        std::env::remove_var("DASH0_MAX_EVENT_PAYLOAD");
        assert_eq!(max_event_payload_size(), 20 * 1024);
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
        assert_eq!(max_event_payload_size(), 20 * 1024);
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
}

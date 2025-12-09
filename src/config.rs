/// Return true if SEND_ON_INVOCATION_END is set to a truthy value.
/// Defaults to false when unset or unrecognized.
pub fn is_send_on_invocation_end() -> bool {
    match std::env::var("SEND_ON_INVOCATION_END") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

pub fn is_auto_instrumented_disabled() -> bool {
    match std::env::var("DISABLE_AUTO_INSTRUMENTATION") {
        Ok(val) => matches!(
            val.as_str(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes" | "y" | "Y"
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_send_on_invocation_end;

    #[test]
    fn defaults_to_false_when_missing() {
        std::env::remove_var("SEND_ON_INVOCATION_END");
        assert!(!is_send_on_invocation_end());
    }

    #[test]
    fn recognizes_truthy_values() {
        for val in ["1", "true", "TRUE", "True", "yes", "YES", "Yes", "y", "Y"] {
            std::env::set_var("SEND_ON_INVOCATION_END", val);
            assert!(is_send_on_invocation_end(), "value {}", val);
        }
    }

    #[test]
    fn false_for_other_values() {
        for val in ["0", "false", "no", "maybe", ""] {
            std::env::set_var("SEND_ON_INVOCATION_END", val);
            assert!(!is_send_on_invocation_end(), "value {}", val);
        }
    }
}

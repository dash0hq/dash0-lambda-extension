use once_cell::sync::Lazy;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use parking_lot::Mutex;

static FUNCTION_ARN: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static ACCOUNT_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static ENV_VAR_ATTRS: Lazy<Mutex<Vec<KeyValue>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn store_function_arn(arn: &str) {
    let mut guard = FUNCTION_ARN.lock();
    if guard.is_none() {
        *guard = Some(arn.to_string());
    }
}

pub fn store_account_id(account_id: &str) {
    let mut guard = ACCOUNT_ID.lock();
    if guard.is_none() {
        *guard = Some(account_id.to_string());
    }
}

pub fn get_function_arn() -> Option<String> {
    let mut guard = FUNCTION_ARN.lock();
    if guard.as_ref().map(|arn| !arn.is_empty()).unwrap_or(false) {
        return guard.clone();
    }

    let account_id = get_account_id().filter(|id| !id.is_empty());
    let region = std::env::var("AWS_REGION").ok().filter(|r| !r.is_empty());
    let function_name = std::env::var("AWS_LAMBDA_FUNCTION_NAME")
        .ok()
        .filter(|name| !name.is_empty());

    match (account_id, region, function_name) {
        (Some(account_id), Some(region), Some(function_name)) => {
            let arn = format!(
                "arn:aws:lambda:{}:{}:function:{}",
                region, account_id, function_name
            );
            *guard = Some(arn.clone());
            Some(arn)
        }
        _ => None,
    }
}

pub fn get_account_id() -> Option<String> {
    ACCOUNT_ID.lock().clone()
}

pub fn init_env_var_attrs() {
    let content = match std::fs::read_to_string("/tmp/dash0_env_vars") {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "[{}] Failed to read /tmp/dash0_env_vars {}",
                crate::log_prefix(),
                e
            );
            return;
        }
    };
    let json: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                "[{}] Failed to parse /tmp/dash0_env_vars: {}",
                crate::log_prefix(),
                e
            );
            return;
        }
    };

    let masked = crate::otlp::masking::mask_env_vars(json);

    let attrs: Vec<KeyValue> = masked
        .into_iter()
        .filter_map(|(key, value)| {
            value.as_str().map(|v| KeyValue {
                key: format!("process.environment_variable.{}", key),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(v.to_string())),
                }),
            })
        })
        .collect();

    *ENV_VAR_ATTRS.lock() = attrs;
}

pub fn get_env_var_attrs() -> Vec<KeyValue> {
    ENV_VAR_ATTRS.lock().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    fn reset_state() {
        FUNCTION_ARN.lock().take();
        ACCOUNT_ID.lock().take();
        ENV_VAR_ATTRS.lock().clear();
        env::remove_var("AWS_REGION");
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
    }

    #[test]
    #[serial]
    fn returns_cached_function_arn_when_set() {
        reset_state();
        let cached = "arn:aws:lambda:us-east-1:123456789012:function:cached-func";
        store_function_arn(cached);

        assert_eq!(get_function_arn(), Some(cached.to_string()));
    }

    #[test]
    #[serial]
    fn constructs_arn_from_env_when_missing() {
        reset_state();
        *ACCOUNT_ID.lock() = Some("111122223333".to_string());
        env::set_var("AWS_REGION", "us-west-2");
        env::set_var("AWS_LAMBDA_FUNCTION_NAME", "my-function");

        let arn = get_function_arn();
        assert_eq!(
            arn.as_deref(),
            Some("arn:aws:lambda:us-west-2:111122223333:function:my-function")
        );

        // Ensure it is cached for subsequent calls even if env vars are removed
        env::remove_var("AWS_REGION");
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
        assert_eq!(
            get_function_arn(),
            Some("arn:aws:lambda:us-west-2:111122223333:function:my-function".to_string())
        );
    }

    #[test]
    #[serial]
    fn returns_none_when_any_component_missing() {
        reset_state();
        // Region present but account id missing
        env::set_var("AWS_REGION", "eu-central-1");
        env::set_var("AWS_LAMBDA_FUNCTION_NAME", "missing-account");

        assert_eq!(get_function_arn(), None);
        assert!(FUNCTION_ARN.lock().is_none());

        // Account present but function name missing
        *ACCOUNT_ID.lock() = Some("444455556666".to_string());
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
        assert_eq!(get_function_arn(), None);
    }

    #[test]
    #[serial]
    fn init_env_var_attrs_masks_sensitive_values() {
        reset_state();
        crate::otlp::masking::init_masking_rules();

        let env_json = serde_json::json!({
            "PATH": "/usr/bin",
            "AWS_SECRET_ACCESS_KEY": "super-secret",
            "API_KEY": "my-api-key",
            "HOME": "/home/user"
        });
        std::fs::write("/tmp/dash0_env_vars", env_json.to_string()).unwrap();

        init_env_var_attrs();

        let attrs = get_env_var_attrs();
        let find_attr = |name: &str| -> Option<String> {
            attrs.iter().find(|kv| kv.key == name).and_then(|kv| {
                kv.value.as_ref().and_then(|v| match &v.value {
                    Some(Value::StringValue(s)) => Some(s.clone()),
                    _ => None,
                })
            })
        };

        assert_eq!(
            find_attr("process.environment_variable.PATH"),
            Some("/usr/bin".to_string())
        );
        assert_eq!(
            find_attr("process.environment_variable.HOME"),
            Some("/home/user".to_string())
        );
        assert_eq!(
            find_attr("process.environment_variable.AWS_SECRET_ACCESS_KEY"),
            Some("****".to_string())
        );
        assert_eq!(
            find_attr("process.environment_variable.API_KEY"),
            Some("****".to_string())
        );

        // cleanup
        let _ = std::fs::remove_file("/tmp/dash0_env_vars");
    }
}

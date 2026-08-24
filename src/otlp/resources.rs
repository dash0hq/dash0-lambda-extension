use opentelemetry_proto::tonic::common::v1::AnyValue;
use opentelemetry_proto::tonic::common::v1::KeyValue;

pub fn try_read_env_from_file(key: &str) -> Option<String> {
    let content = std::fs::read_to_string("/tmp/dash0_env_vars").ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Reads a variable the extension process can see directly.
///
/// Lambda hands extensions the function's configured environment, so this is the
/// value the user set. It never contains what `opt/shared.sh` adds, because that
/// script exports into the runtime process, which starts after this one.
fn read_configured_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Sets a key, replacing an earlier value in place so the last writer wins and
/// the resource never carries the same key twice.
fn set_attribute(attributes: &mut Vec<(String, String)>, key: &str, value: String) {
    match attributes.iter_mut().find(|(existing, _)| existing == key) {
        Some((_, existing)) => *existing = value,
        None => attributes.push((key.to_string(), value)),
    }
}

/// Merges a serialized `key=value,key=value` list. Later pairs overwrite earlier
/// ones, which is what the wrapper's own layout requires: `setup_otel_env`
/// prepends the Lambda attributes and appends the user's value after them.
fn merge_serialized_attributes(attributes: &mut Vec<(String, String)>, serialized: &str) {
    for pair in serialized.split(',') {
        if let Some((key, value)) = pair.split_once('=') {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            set_attribute(attributes, key, value.trim().to_string());
        }
    }
}

pub fn get_resources_attributes() -> Vec<KeyValue> {
    use crate::otlp::attributes::*;

    let mut attributes: Vec<(String, String)> = vec![
        (CLOUD_PLATFORM.to_string(), "aws_lambda".to_string()),
        (
            CLOUD_RESOURCE_ID.to_string(),
            crate::state::global::get_function_arn().unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            CLOUD_ACCOUNT_ID.to_string(),
            crate::state::global::get_account_id().unwrap_or_else(|| "unknown".to_string()),
        ),
    ];

    // The runtime process sees the attributes `opt/shared.sh` adds, including
    // faas.extension.git_hash, followed by the user's value. `write_env_vars`
    // dumps that combined value to /tmp/dash0_env_vars for this process to read.
    if let Some(from_wrapper) = try_read_env_from_file("OTEL_RESOURCE_ATTRIBUTES") {
        merge_serialized_attributes(&mut attributes, &from_wrapper);
    }

    // Merge the configured value last so the user wins on a conflicting key. Do
    // not stop at the first source: reading only this one drops every attribute
    // the wrapper adds.
    if let Some(from_user) = read_configured_env("OTEL_RESOURCE_ATTRIBUTES") {
        merge_serialized_attributes(&mut attributes, &from_user);
    }

    // OTEL_SERVICE_NAME wins over a service.name resource attribute, per spec.
    // But `opt/shared.sh` writes a fallback OTEL_SERVICE_NAME=$AWS_LAMBDA_FUNCTION_NAME
    // into the file whenever the user hasn't set it directly, so a file-sourced
    // value here isn't necessarily an explicit override -- only fall back to it
    // when no service.name has already been merged in from OTEL_RESOURCE_ATTRIBUTES.
    let has_service_name_attribute = attributes.iter().any(|(key, _)| key == SERVICE_NAME);
    let service_name = read_configured_env("OTEL_SERVICE_NAME").or_else(|| {
        if has_service_name_attribute {
            None
        } else {
            try_read_env_from_file("OTEL_SERVICE_NAME")
        }
    });
    match service_name {
        Some(name) => set_attribute(&mut attributes, SERVICE_NAME, name),
        None => {
            if !has_service_name_attribute {
                set_attribute(&mut attributes, SERVICE_NAME, "unknown_service".to_string());
            }
        }
    }

    attributes
        .into_iter()
        .map(|(key, value)| KeyValue {
            key,
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value),
                ),
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use serde_json::json;

    fn get_string_value(any_value: &Option<AnyValue>) -> Option<String> {
        match any_value {
            Some(AnyValue {
                value: Some(Value::StringValue(s)),
            }) => Some(s.clone()),
            _ => None,
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_get_resources_attributes_structure() {
        let expected_service_name = "test-service-name";
        std::env::set_var("OTEL_SERVICE_NAME", expected_service_name);

        let attributes = get_resources_attributes();

        let keys: Vec<String> = attributes.iter().map(|kv| kv.key.clone()).collect();
        assert!(keys.contains(&"cloud.resource_id".to_string()));
        assert!(keys.contains(&"cloud.account.id".to_string()));
        assert!(keys.contains(&"service.name".to_string()));

        let service_name_attr = attributes
            .iter()
            .find(|kv| kv.key == "service.name")
            .unwrap();

        assert_eq!(
            get_string_value(&service_name_attr.value),
            Some(expected_service_name.to_string())
        );

        std::env::remove_var("OTEL_SERVICE_NAME");
    }

    #[test]
    #[serial_test::serial]
    fn test_get_resources_attributes_from_file_fallback() {
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");

        let file_path = "/tmp/dash0_env_vars";
        let expected_service_name = "service-from-file";
        let expected_resource_attrs = "key1=value1,key2=value2";
        let content = json!({
            "OTEL_SERVICE_NAME": expected_service_name,
            "OTEL_RESOURCE_ATTRIBUTES": expected_resource_attrs
        })
        .to_string();
        std::fs::write(file_path, content).expect("Failed to write mock file");

        let attributes = get_resources_attributes();

        std::fs::remove_file(file_path).expect("Failed to cleanup mock file");

        let service_name_attr = attributes
            .iter()
            .find(|kv| kv.key == "service.name")
            .unwrap();

        assert_eq!(
            get_string_value(&service_name_attr.value),
            Some(expected_service_name.to_string())
        );

        let key1_attr = attributes.iter().find(|kv| kv.key == "key1").unwrap();
        assert_eq!(
            get_string_value(&key1_attr.value),
            Some("value1".to_string())
        );

        let key2_attr = attributes.iter().find(|kv| kv.key == "key2").unwrap();
        assert_eq!(
            get_string_value(&key2_attr.value),
            Some("value2".to_string())
        );
    }

    // The value the runtime process sees: `setup_otel_env` prepends the Lambda and
    // extension attributes, then appends whatever the function configured.
    const WRAPPER_ATTRS: &str = "cloud.region=us-east-1,cloud.provider=aws,\
faas.name=repro-fn,faas.version=$LATEST,faas.instance=2026/08/21/[$LATEST]abc,\
faas.extension.git_hash=7ca4b1d6ef0cf0f17c7101690ea55f94ce0d2152,\
deployment.environment.name=preview,service.namespace=slfinrtl";

    fn attributes_map() -> std::collections::HashMap<String, String> {
        get_resources_attributes()
            .into_iter()
            .map(|kv| (kv.key, get_string_value(&kv.value).unwrap_or_default()))
            .collect()
    }

    fn write_wrapper_file(resource_attributes: &str) {
        std::fs::write(
            "/tmp/dash0_env_vars",
            json!({ "OTEL_RESOURCE_ATTRIBUTES": resource_attributes }).to_string(),
        )
        .expect("Failed to write mock file");
    }

    // A function that configures OTEL_RESOURCE_ATTRIBUTES used to hide every
    // attribute the wrapper adds, because the extension process only ever sees the
    // configured value and stopped there.
    #[test]
    #[serial_test::serial]
    fn test_configured_attributes_do_not_hide_wrapper_attributes() {
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::set_var(
            "OTEL_RESOURCE_ATTRIBUTES",
            "deployment.environment.name=preview,service.namespace=slfinrtl",
        );
        write_wrapper_file(WRAPPER_ATTRS);

        let attributes = attributes_map();

        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");
        std::fs::remove_file("/tmp/dash0_env_vars").expect("Failed to cleanup mock file");

        assert_eq!(
            attributes
                .get("faas.extension.git_hash")
                .map(String::as_str),
            Some("7ca4b1d6ef0cf0f17c7101690ea55f94ce0d2152")
        );
        assert_eq!(
            attributes.get("faas.version").map(String::as_str),
            Some("$LATEST")
        );
        assert_eq!(
            attributes.get("faas.instance").map(String::as_str),
            Some("2026/08/21/[$LATEST]abc")
        );
        assert_eq!(
            attributes.get("faas.name").map(String::as_str),
            Some("repro-fn")
        );
        // The configured attributes survive too.
        assert_eq!(
            attributes.get("service.namespace").map(String::as_str),
            Some("slfinrtl")
        );
        assert_eq!(
            attributes
                .get("deployment.environment.name")
                .map(String::as_str),
            Some("preview")
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_configured_value_wins_on_conflicting_key() {
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::set_var(
            "OTEL_RESOURCE_ATTRIBUTES",
            "faas.version=pinned-by-user,cloud.platform=configured-platform",
        );
        write_wrapper_file(WRAPPER_ATTRS);

        let attributes = get_resources_attributes();

        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");
        std::fs::remove_file("/tmp/dash0_env_vars").expect("Failed to cleanup mock file");

        let value_of = |key: &str| {
            let matches: Vec<String> = attributes
                .iter()
                .filter(|kv| kv.key == key)
                .filter_map(|kv| get_string_value(&kv.value))
                .collect();
            // A duplicate key would leave the winner up to the backend.
            assert_eq!(matches.len(), 1, "expected one {} attribute", key);
            matches[0].clone()
        };

        assert_eq!(value_of("faas.version"), "pinned-by-user");
        assert_eq!(value_of("cloud.platform"), "configured-platform");
        assert_eq!(
            value_of("faas.extension.git_hash"),
            "7ca4b1d6ef0cf0f17c7101690ea55f94ce0d2152"
        );
    }

    // service.name has its own precedence rule: OTEL_SERVICE_NAME beats a
    // service.name resource attribute from either source.
    #[test]
    #[serial_test::serial]
    fn test_service_name_precedence() {
        std::env::set_var("OTEL_SERVICE_NAME", "from-otel-service-name");
        std::env::set_var("OTEL_RESOURCE_ATTRIBUTES", "service.name=from-attributes");
        write_wrapper_file(WRAPPER_ATTRS);

        let with_service_name_env = attributes_map();

        std::env::remove_var("OTEL_SERVICE_NAME");
        let without_service_name_env = attributes_map();

        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");
        std::fs::remove_file("/tmp/dash0_env_vars").expect("Failed to cleanup mock file");

        assert_eq!(
            with_service_name_env
                .get("service.name")
                .map(String::as_str),
            Some("from-otel-service-name")
        );
        assert_eq!(
            without_service_name_env
                .get("service.name")
                .map(String::as_str),
            Some("from-attributes")
        );
    }

    // `opt/shared.sh` always writes OTEL_SERVICE_NAME into the file -- either the
    // user's own value, or (when the user never set it) a fallback to
    // $AWS_LAMBDA_FUNCTION_NAME. That fallback must not clobber a service.name the
    // user configured via OTEL_RESOURCE_ATTRIBUTES instead of OTEL_SERVICE_NAME.
    #[test]
    #[serial_test::serial]
    fn test_file_service_name_fallback_does_not_override_configured_resource_attribute() {
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::set_var("OTEL_RESOURCE_ATTRIBUTES", "service.name=from-attributes");
        std::fs::write(
            "/tmp/dash0_env_vars",
            json!({
                "OTEL_RESOURCE_ATTRIBUTES": WRAPPER_ATTRS,
                // shared.sh's fallback -- the user never configured this directly.
                "OTEL_SERVICE_NAME": "repro-fn"
            })
            .to_string(),
        )
        .expect("Failed to write mock file");

        let attributes = attributes_map();

        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");
        std::fs::remove_file("/tmp/dash0_env_vars").expect("Failed to cleanup mock file");

        assert_eq!(
            attributes.get("service.name").map(String::as_str),
            Some("from-attributes")
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_malformed_pairs_are_skipped() {
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");
        write_wrapper_file(",,no-equals-sign,=orphan-value, spaced.key = spaced value ,ok=1");

        let attributes = attributes_map();

        std::fs::remove_file("/tmp/dash0_env_vars").expect("Failed to cleanup mock file");

        assert_eq!(attributes.get("ok").map(String::as_str), Some("1"));
        assert_eq!(
            attributes.get("spaced.key").map(String::as_str),
            Some("spaced value")
        );
        assert!(!attributes.contains_key("no-equals-sign"));
        assert!(!attributes.contains_key(""));
        assert_eq!(
            attributes.get("service.name").map(String::as_str),
            Some("unknown_service")
        );
    }
}

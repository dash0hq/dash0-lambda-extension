use opentelemetry_proto::tonic::common::v1::AnyValue;
use opentelemetry_proto::tonic::common::v1::KeyValue;

pub fn try_read_env_from_file(key: &str) -> Option<String> {
    let content = std::fs::read_to_string("/tmp/dash0_env_vars").ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn get_resources_attributes() -> Vec<KeyValue> {
    use crate::otlp::attributes::*;
    let mut attributes = vec![
        KeyValue {
            key: CLOUD_PLATFORM.to_string(),
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        "aws_lambda".to_string(),
                    ),
                ),
            }),
        },
        KeyValue {
            key: CLOUD_RESOURCE_ID.to_string(),
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        crate::state::global::get_function_arn()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ),
            }),
        },
        KeyValue {
            key: CLOUD_ACCOUNT_ID.to_string(),
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        crate::state::global::get_account_id()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ),
            }),
        },
        KeyValue {
            key: SERVICE_NAME.to_string(),
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        std::env::var("OTEL_SERVICE_NAME")
                            .ok()
                            .filter(|v| !v.is_empty())
                            .or_else(|| try_read_env_from_file("OTEL_SERVICE_NAME"))
                            .unwrap_or_else(|| "unknown_service".to_string()),
                    ),
                ),
            }),
        },
    ];

    let resource_attributes = std::env::var("OTEL_RESOURCE_ATTRIBUTES")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| try_read_env_from_file("OTEL_RESOURCE_ATTRIBUTES"))
        .unwrap_or_default();

    for pair in resource_attributes.split(',') {
        if let Some((key, value)) = pair.split_once('=') {
            attributes.push(KeyValue {
                key: key.trim().to_string(),
                value: Some(AnyValue {
                    value: Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                            value.trim().to_string(),
                        ),
                    ),
                }),
            });
        }
    }

    attributes
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
        assert!(keys.contains(&"cloud.resource.id".to_string()));
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
}

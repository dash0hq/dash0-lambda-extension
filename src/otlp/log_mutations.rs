use crate::config::is_auto_instrumented_disabled;
use crate::state::invocation_data::TelemetryLog;
use crate::state::invocation_entry;
use crate::util::parsers::{get_span_id_from_invocation_id, get_trace_id_from_invocation_id};
use chrono::DateTime;
use opentelemetry_proto::tonic::common::v1::AnyValue;
use opentelemetry_proto::tonic::logs::v1::LogRecord;

/// Formats a platform.report log record into a CloudWatch-style REPORT message.
fn format_platform_report_message(record: &serde_json::Map<String, serde_json::Value>) -> String {
    let request_id = record
        .get("requestId")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let metrics = record.get("metrics").and_then(|v| v.as_object());

    let duration_ms = metrics
        .and_then(|m| m.get("durationMs"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let billed_duration_ms = metrics
        .and_then(|m| m.get("billedDurationMs"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let memory_size_mb = metrics
        .and_then(|m| m.get("memorySizeMB"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let max_memory_used_mb = metrics
        .and_then(|m| m.get("maxMemoryUsedMB"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let init_duration_ms = metrics
        .and_then(|m| m.get("initDurationMs"))
        .and_then(|v| v.as_f64());

    let status = record.get("status").and_then(|v| v.as_str());
    let error_type = record.get("errorType").and_then(|v| v.as_str());

    // Format the REPORT message like CloudWatch
    let mut report_message = format!(
        "REPORT RequestId: {}\tDuration: {:.2} ms\tBilled Duration: {} ms\tMemory Size: {} MB\tMax Memory Used: {} MB",
        request_id, duration_ms, billed_duration_ms, memory_size_mb, max_memory_used_mb
    );

    if let Some(init_duration) = init_duration_ms {
        report_message.push_str(&format!("\tInit Duration: {:.2} ms", init_duration));
    }

    // Add status if it's not "success"
    if let Some(status_str) = status {
        if status_str != "success" {
            report_message.push_str(&format!("\tStatus: {}", status_str));
            if status_str != "timeout" {
                if let Some(error_type_str) = error_type {
                    report_message.push_str(&format!("\tError Type: {}", error_type_str));
                }
            }
        }
    }

    report_message
}

/// Formats a platform.start log record into a CloudWatch-style START message.
fn format_platform_start_message(record: &serde_json::Map<String, serde_json::Value>) -> String {
    let request_id = record
        .get("requestId")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let version = record
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    format!("START RequestId: {} Version: {}", request_id, version)
}

/// Formats a platform.runtimeDone log record into a CloudWatch-style END message.
fn format_platform_runtime_done_message(
    record: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let request_id = record
        .get("requestId")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    format!("END RequestId: {}", request_id)
}

pub fn map_logs_to_otlp(logs: &[TelemetryLog], is_invocation_end: bool) -> Vec<LogRecord> {
    let mut log_records = Vec::new();
    for log in logs {
        // Determine the log body based on log type
        let body_message = if log.r#type == "function" {
            match &log.record {
                serde_json::Value::String(s) => Some(s.clone()),
                _ => None,
            }
        } else if log.r#type == "platform.report" {
            // Handle platform.report logs
            if let serde_json::Value::Object(record) = &log.record {
                Some(format_platform_report_message(record))
            } else {
                None
            }
        } else if log.r#type == "platform.start" {
            // Handle platform.start logs
            if let serde_json::Value::Object(record) = &log.record {
                Some(format_platform_start_message(record))
            } else {
                None
            }
        } else if log.r#type == "platform.runtimeDone" {
            // Handle platform.runtimeDone logs
            if let serde_json::Value::Object(record) = &log.record {
                Some(format_platform_runtime_done_message(record))
            } else {
                None
            }
        } else {
            None
        };

        // Skip if we couldn't extract a body message
        let body_message = match body_message {
            Some(msg) => msg,
            None => continue,
        };

        // Determine if this is a platform log (START, END, REPORT)
        let is_platform_log = log.r#type == "platform.start"
            || log.r#type == "platform.runtimeDone"
            || log.r#type == "platform.report";

        // Common processing for all log types
        let timestamp_nanos = if let Ok(dt) = DateTime::parse_from_rfc3339(&log.time) {
            dt.timestamp_nanos_opt().unwrap_or(0) as u64
        } else {
            0
        };

        let mut attributes = Vec::new();
        let mut trace_id = vec![0u8; 16];
        let mut span_id = vec![0u8; 8];

        if let Some(invocation_id) = &log.invocation_id {
            attributes.push(opentelemetry_proto::tonic::common::v1::KeyValue {
                key: "faas.invocation_id".to_string(),
                value: Some(AnyValue {
                    value: Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                            invocation_id.clone(),
                        ),
                    ),
                }),
            });

            if let Some(entry) = invocation_entry::get(invocation_id) {
                if let Some(ref tid_hex) = entry.trace_id {
                    if let Ok(tid) = hex::decode(tid_hex) {
                        if tid.len() == 16 {
                            trace_id = tid;
                        }
                    }
                }
                if let Some(ref sid_hex) = entry.span_id {
                    if let Ok(sid) = hex::decode(sid_hex) {
                        if sid.len() == 8 {
                            span_id = sid;
                        }
                    }
                }
            } else if is_invocation_end || is_auto_instrumented_disabled() {
                trace_id = get_trace_id_from_invocation_id(invocation_id);
                span_id = get_span_id_from_invocation_id(invocation_id);
            } else {
                tracing::info!(
                    "[{}] trace/span ids not found for invocation_id {}, putting back to store",
                    crate::log_prefix(),
                    invocation_id
                );
                invocation_entry::store_telemetry_logs(vec![log.clone()]);
                continue;
            }
        }

        let log_record = LogRecord {
            time_unix_nano: timestamp_nanos,
            observed_time_unix_nano: timestamp_nanos,
            severity_number: if is_platform_log { 9 } else { 0 }, // 9 = INFO
            severity_text: if is_platform_log {
                "INFO".to_string()
            } else {
                String::new()
            },
            body: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        body_message,
                    ),
                ),
            }),
            attributes,
            trace_id,
            span_id,
            ..Default::default()
        };
        log_records.push(log_record);
    }
    log_records
}

pub fn try_read_env_from_file(key: &str) -> Option<String> {
    let content = std::fs::read_to_string("/tmp/lumigo_env_vars").ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn get_resources_attributes() -> Vec<opentelemetry_proto::tonic::common::v1::KeyValue> {
    let mut attributes = vec![
        opentelemetry_proto::tonic::common::v1::KeyValue {
            key: "cloud.resource.id".to_string(),
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        crate::state::global::get_function_arn()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ),
            }),
        },
        opentelemetry_proto::tonic::common::v1::KeyValue {
            key: "cloud.account.id".to_string(),
            value: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        crate::state::global::get_account_id()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ),
            }),
        },
        opentelemetry_proto::tonic::common::v1::KeyValue {
            key: "service.name".to_string(),
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
            attributes.push(opentelemetry_proto::tonic::common::v1::KeyValue {
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
    use serial_test::serial;

    fn get_string_value(any_value: &Option<AnyValue>) -> Option<String> {
        match any_value {
            Some(AnyValue {
                value: Some(Value::StringValue(s)),
            }) => Some(s.clone()),
            _ => None,
        }
    }

    #[test]
    fn test_map_logs_happy_path() {
        let logs = vec![TelemetryLog {
            time: "2023-10-26T12:00:00.000Z".to_string(),
            r#type: "function".to_string(),
            record: json!("Hello World"),
            invocation_id: Some("inv-123".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        // Verify Body
        assert_eq!(get_string_value(&log.body), Some("Hello World".to_string()));

        // Verify Timestamp (2023-10-26T12:00:00.000Z is 1698321600 seconds)
        assert_eq!(log.time_unix_nano, 1698321600000000000);

        // Verify Invocation ID Attribute
        assert_eq!(log.attributes.len(), 1);
        assert_eq!(log.attributes[0].key, "faas.invocation_id");
        assert_eq!(
            get_string_value(&log.attributes[0].value),
            Some("inv-123".to_string())
        );
    }

    #[test]
    fn test_map_logs_ignores_non_function_type() {
        let logs = vec![TelemetryLog {
            time: "2023-10-26T12:00:00.000Z".to_string(),
            r#type: "platform.start".to_string(),
            record: json!("Start"),
            invocation_id: Some("inv-123".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);
        assert!(result.is_empty());
    }

    #[test]
    fn test_map_logs_ignores_non_string_record() {
        let logs = vec![TelemetryLog {
            time: "2023-10-26T12:00:00.000Z".to_string(),
            r#type: "function".to_string(),
            record: json!({"foo": "bar"}), // Object instead of string
            invocation_id: Some("inv-123".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);
        assert!(result.is_empty());
    }

    #[test]
    fn test_map_logs_invalid_time_defaults_to_zero() {
        let logs = vec![TelemetryLog {
            time: "invalid-time".to_string(),
            r#type: "function".to_string(),
            record: json!("msg"),
            invocation_id: Some("inv-123".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].time_unix_nano, 0);
    }

    #[test]
    fn test_map_logs_without_invocation_id() {
        let logs = vec![TelemetryLog {
            time: "2023-10-26T12:00:00.000Z".to_string(),
            r#type: "function".to_string(),
            record: json!("msg"),
            invocation_id: None,
        }];

        let result = map_logs_to_otlp(&logs, true);
        assert_eq!(result.len(), 1);
        assert!(result[0].attributes.is_empty());
    }

    #[test]
    fn test_map_logs_mixed_batch() {
        let logs = vec![
            TelemetryLog {
                time: "2023-10-26T12:00:00.000Z".to_string(),
                r#type: "function".to_string(),
                record: json!("Valid 1"),
                invocation_id: Some("1".to_string()),
            },
            TelemetryLog {
                time: "2023-10-26T12:00:01.000Z".to_string(),
                r#type: "platform.end".to_string(),
                record: json!("Ignored"),
                invocation_id: Some("2".to_string()),
            },
            TelemetryLog {
                time: "2023-10-26T12:00:02.000Z".to_string(),
                r#type: "function".to_string(),
                record: json!("Valid 2"),
                invocation_id: Some("3".to_string()),
            },
        ];

        let result = map_logs_to_otlp(&logs, true);
        assert_eq!(result.len(), 2);
        assert_eq!(
            get_string_value(&result[0].body),
            Some("Valid 1".to_string())
        );
        assert_eq!(
            get_string_value(&result[1].body),
            Some("Valid 2".to_string())
        );
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

        // Cleanup
        std::env::remove_var("OTEL_SERVICE_NAME");
    }
    #[test]
    #[serial_test::serial]
    fn test_get_resources_attributes_from_file_fallback() {
        // Ensure env var is unset
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");

        // Write mock file
        let file_path = "/tmp/lumigo_env_vars";
        let expected_service_name = "service-from-file";
        let expected_resource_attrs = "key1=value1,key2=value2";
        let content = json!({
            "OTEL_SERVICE_NAME": expected_service_name,
            "OTEL_RESOURCE_ATTRIBUTES": expected_resource_attrs
        })
        .to_string();
        std::fs::write(file_path, content).expect("Failed to write mock file");

        let attributes = get_resources_attributes();

        // Cleanup file
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

    #[test]
    fn test_map_logs_with_trace_and_span_id_from_store() {
        use crate::state::invocation_entry;

        let invocation_id = "inv-with-trace";
        let trace_id_hex = "5b8eff129842a1b9c9283745a23f54b1";
        let span_id_hex = "023f54b19283745a";

        // Store the mapping
        invocation_entry::update(invocation_id, |entry| {
            entry.trace_id = Some(trace_id_hex.to_string());
            entry.span_id = Some(span_id_hex.to_string());
        });

        let logs = vec![TelemetryLog {
            time: "2023-10-26T12:00:00.000Z".to_string(),
            r#type: "function".to_string(),
            record: json!("Log with trace"),
            invocation_id: Some(invocation_id.to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        // Verify trace_id
        let expected_trace_id = hex::decode(trace_id_hex).unwrap();
        assert_eq!(log.trace_id, expected_trace_id);

        // Verify span_id
        let expected_span_id = hex::decode(span_id_hex).unwrap();
        assert_eq!(log.span_id, expected_span_id);
    }

    #[test]
    fn test_map_logs_platform_report_with_init_duration() {
        let logs = vec![TelemetryLog {
            time: "2025-12-07T12:09:10.254Z".to_string(),
            r#type: "platform.report".to_string(),
            record: json!({
                "requestId": "3b496829-c13e-47fa-83ef-a779f19adfc3",
                "metrics": {
                    "durationMs": 3729.809,
                    "billedDurationMs": 4747,
                    "memorySizeMB": 128,
                    "maxMemoryUsedMB": 94,
                    "initDurationMs": 1016.77
                }
            }),
            invocation_id: Some("inv-report-1".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert!(body.starts_with("REPORT RequestId: 3b496829-c13e-47fa-83ef-a779f19adfc3"));
        assert!(body.contains("Duration: 3729.81 ms"));
        assert!(body.contains("Billed Duration: 4747 ms"));
        assert!(body.contains("Memory Size: 128 MB"));
        assert!(body.contains("Max Memory Used: 94 MB"));
        assert!(body.contains("Init Duration: 1016.77 ms"));

        // Verify invocation_id attribute
        assert_eq!(log.attributes.len(), 1);
        assert_eq!(log.attributes[0].key, "faas.invocation_id");
        assert_eq!(
            get_string_value(&log.attributes[0].value),
            Some("inv-report-1".to_string())
        );
    }

    #[test]
    fn test_map_logs_platform_report_without_init_duration() {
        let logs = vec![TelemetryLog {
            time: "2025-12-07T12:09:10.254Z".to_string(),
            r#type: "platform.report".to_string(),
            record: json!({
                "requestId": "test-request-id",
                "metrics": {
                    "durationMs": 100.5,
                    "billedDurationMs": 101,
                    "memorySizeMB": 256,
                    "maxMemoryUsedMB": 128
                }
            }),
            invocation_id: Some("inv-report-2".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert!(body.starts_with("REPORT RequestId: test-request-id"));
        assert!(body.contains("Duration: 100.50 ms"));
        assert!(body.contains("Billed Duration: 101 ms"));
        assert!(body.contains("Memory Size: 256 MB"));
        assert!(body.contains("Max Memory Used: 128 MB"));
        assert!(
            !body.contains("Init Duration"),
            "should not contain Init Duration when not present"
        );
    }

    #[test]
    fn test_map_logs_platform_report_with_timeout_status() {
        let logs = vec![TelemetryLog {
            time: "2025-12-08T10:24:35.486Z".to_string(),
            r#type: "platform.report".to_string(),
            record: json!({
                "requestId": "eee48020-0cfd-4761-be0c-3eaa76fd0a31",
                "metrics": {
                    "durationMs": 10000.0,
                    "billedDurationMs": 11029,
                    "memorySizeMB": 128,
                    "maxMemoryUsedMB": 94,
                    "initDurationMs": 1028.322
                },
                "status": "timeout"
            }),
            invocation_id: Some("inv-timeout-1".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert!(body.starts_with("REPORT RequestId: eee48020-0cfd-4761-be0c-3eaa76fd0a31"));
        assert!(body.contains("Duration: 10000.00 ms"));
        assert!(body.contains("Billed Duration: 11029 ms"));
        assert!(body.contains("Memory Size: 128 MB"));
        assert!(body.contains("Max Memory Used: 94 MB"));
        assert!(body.contains("Init Duration: 1028.32 ms"));
        assert!(body.contains("Status: timeout"));

        // Verify invocation_id attribute
        assert_eq!(log.attributes.len(), 1);
        assert_eq!(log.attributes[0].key, "faas.invocation_id");
        assert_eq!(
            get_string_value(&log.attributes[0].value),
            Some("inv-timeout-1".to_string())
        );

        // Verify it has INFO severity (platform logs should have INFO severity)
        assert_eq!(log.severity_number, 9); // INFO
        assert_eq!(log.severity_text, "INFO");
    }

    #[test]
    fn test_map_logs_platform_report_with_success_status() {
        let logs = vec![TelemetryLog {
            time: "2025-12-08T10:24:35.486Z".to_string(),
            r#type: "platform.report".to_string(),
            record: json!({
                "requestId": "success-request-id",
                "metrics": {
                    "durationMs": 150.5,
                    "billedDurationMs": 151,
                    "memorySizeMB": 128,
                    "maxMemoryUsedMB": 64,
                    "initDurationMs": 100.0
                },
                "status": "success"
            }),
            invocation_id: Some("inv-success-1".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert!(body.starts_with("REPORT RequestId: success-request-id"));
        assert!(body.contains("Duration: 150.50 ms"));
        assert!(body.contains("Billed Duration: 151 ms"));
        assert!(body.contains("Memory Size: 128 MB"));
        assert!(body.contains("Max Memory Used: 64 MB"));
        assert!(body.contains("Init Duration: 100.00 ms"));
        // Status should NOT be included when it's "success"
        assert!(
            !body.contains("Status:"),
            "should not contain Status when status is success"
        );
    }

    #[test]
    fn test_map_logs_platform_report_with_error_status() {
        let logs = vec![TelemetryLog {
            time: "2025-12-08T10:24:35.486Z".to_string(),
            r#type: "platform.report".to_string(),
            record: json!({
                "requestId": "error-request-id",
                "metrics": {
                    "durationMs": 250.75,
                    "billedDurationMs": 251,
                    "memorySizeMB": 256,
                    "maxMemoryUsedMB": 128
                },
                "status": "error",
                "errorType": "Runtime.OutOfMemory"
            }),
            invocation_id: Some("inv-error-1".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert!(body.starts_with("REPORT RequestId: error-request-id"));
        assert!(body.contains("Duration: 250.75 ms"));
        assert!(body.contains("Billed Duration: 251 ms"));
        assert!(body.contains("Memory Size: 256 MB"));
        assert!(body.contains("Max Memory Used: 128 MB"));
        assert!(body.contains("Status: error"));
        assert!(body.contains("Error Type: Runtime.OutOfMemory"));
    }

    #[test]
    fn test_map_logs_mixed_function_and_platform_report() {
        let logs = vec![
            TelemetryLog {
                time: "2025-12-07T12:09:09.000Z".to_string(),
                r#type: "function".to_string(),
                record: json!("User log message"),
                invocation_id: Some("inv-123".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:10.254Z".to_string(),
                r#type: "platform.report".to_string(),
                record: json!({
                    "requestId": "inv-123",
                    "metrics": {
                        "durationMs": 1000.0,
                        "billedDurationMs": 1000,
                        "memorySizeMB": 512,
                        "maxMemoryUsedMB": 256,
                        "initDurationMs": 500.0
                    }
                }),
                invocation_id: Some("inv-123".to_string()),
            },
        ];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 2);

        // First log should be the function log
        assert_eq!(
            get_string_value(&result[0].body),
            Some("User log message".to_string())
        );

        // Second log should be the REPORT
        let report_body = get_string_value(&result[1].body).expect("report body should be present");
        assert!(report_body.starts_with("REPORT RequestId: inv-123"));
        assert!(report_body.contains("Duration: 1000.00 ms"));
    }

    #[test]
    fn test_map_logs_platform_start() {
        let logs = vec![TelemetryLog {
            time: "2025-12-07T12:09:06.523Z".to_string(),
            r#type: "platform.start".to_string(),
            record: json!({
                "requestId": "3b496829-c13e-47fa-83ef-a779f19adfc3",
                "version": "$LATEST"
            }),
            invocation_id: Some("inv-start-1".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert_eq!(
            body,
            "START RequestId: 3b496829-c13e-47fa-83ef-a779f19adfc3 Version: $LATEST"
        );

        // Verify invocation_id attribute
        assert_eq!(log.attributes.len(), 1);
        assert_eq!(log.attributes[0].key, "faas.invocation_id");
        assert_eq!(
            get_string_value(&log.attributes[0].value),
            Some("inv-start-1".to_string())
        );
    }

    #[test]
    fn test_map_logs_platform_runtime_done() {
        let logs = vec![TelemetryLog {
            time: "2025-12-07T12:09:10.252Z".to_string(),
            r#type: "platform.runtimeDone".to_string(),
            record: json!({
                "requestId": "3b496829-c13e-47fa-83ef-a779f19adfc3",
                "status": "success",
                "spans": [{
                    "name": "responseLatency",
                    "start": "2025-12-07T12:09:06.523Z",
                    "durationMs": 3049.951
                }],
                "metrics": {
                    "durationMs": 3729.3,
                    "producedBytes": 53
                }
            }),
            invocation_id: Some("inv-end-1".to_string()),
        }];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 1);
        let log = &result[0];

        let body = get_string_value(&log.body).expect("body should be present");
        assert_eq!(body, "END RequestId: 3b496829-c13e-47fa-83ef-a779f19adfc3");

        // Verify invocation_id attribute
        assert_eq!(log.attributes.len(), 1);
        assert_eq!(log.attributes[0].key, "faas.invocation_id");
        assert_eq!(
            get_string_value(&log.attributes[0].value),
            Some("inv-end-1".to_string())
        );
    }

    #[test]
    fn test_map_logs_complete_invocation_lifecycle() {
        let logs = vec![
            TelemetryLog {
                time: "2025-12-07T12:09:06.523Z".to_string(),
                r#type: "platform.start".to_string(),
                record: json!({
                    "requestId": "test-lifecycle",
                    "version": "$LATEST"
                }),
                invocation_id: Some("test-lifecycle".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:07.000Z".to_string(),
                r#type: "function".to_string(),
                record: json!("Processing request..."),
                invocation_id: Some("test-lifecycle".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:10.252Z".to_string(),
                r#type: "platform.runtimeDone".to_string(),
                record: json!({
                    "requestId": "test-lifecycle",
                    "status": "success"
                }),
                invocation_id: Some("test-lifecycle".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:10.254Z".to_string(),
                r#type: "platform.report".to_string(),
                record: json!({
                    "requestId": "test-lifecycle",
                    "metrics": {
                        "durationMs": 3729.809,
                        "billedDurationMs": 4747,
                        "memorySizeMB": 128,
                        "maxMemoryUsedMB": 94
                    }
                }),
                invocation_id: Some("test-lifecycle".to_string()),
            },
        ];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 4);

        // Verify START
        let start_body = get_string_value(&result[0].body).expect("start body should be present");
        assert!(start_body.starts_with("START RequestId: test-lifecycle"));

        // Verify function log
        assert_eq!(
            get_string_value(&result[1].body),
            Some("Processing request...".to_string())
        );

        // Verify END
        let end_body = get_string_value(&result[2].body).expect("end body should be present");
        assert_eq!(end_body, "END RequestId: test-lifecycle");

        // Verify REPORT
        let report_body = get_string_value(&result[3].body).expect("report body should be present");
        assert!(report_body.starts_with("REPORT RequestId: test-lifecycle"));
    }

    #[test]
    fn test_platform_logs_have_info_severity() {
        let logs = vec![
            TelemetryLog {
                time: "2025-12-07T12:09:06.523Z".to_string(),
                r#type: "platform.start".to_string(),
                record: json!({
                    "requestId": "test-severity",
                    "version": "$LATEST"
                }),
                invocation_id: Some("test-severity".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:07.000Z".to_string(),
                r#type: "function".to_string(),
                record: json!("User log"),
                invocation_id: Some("test-severity".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:10.252Z".to_string(),
                r#type: "platform.runtimeDone".to_string(),
                record: json!({
                    "requestId": "test-severity",
                    "status": "success"
                }),
                invocation_id: Some("test-severity".to_string()),
            },
            TelemetryLog {
                time: "2025-12-07T12:09:10.254Z".to_string(),
                r#type: "platform.report".to_string(),
                record: json!({
                    "requestId": "test-severity",
                    "metrics": {
                        "durationMs": 100.0,
                        "billedDurationMs": 100,
                        "memorySizeMB": 128,
                        "maxMemoryUsedMB": 64
                    }
                }),
                invocation_id: Some("test-severity".to_string()),
            },
        ];

        let result = map_logs_to_otlp(&logs, true);

        assert_eq!(result.len(), 4);

        // Verify START has INFO severity
        assert_eq!(result[0].severity_number, 9); // INFO
        assert_eq!(result[0].severity_text, "INFO");

        // Verify function log has no severity (0)
        assert_eq!(result[1].severity_number, 0);
        assert_eq!(result[1].severity_text, "");

        // Verify END has INFO severity
        assert_eq!(result[2].severity_number, 9); // INFO
        assert_eq!(result[2].severity_text, "INFO");

        // Verify REPORT has INFO severity
        assert_eq!(result[3].severity_number, 9); // INFO
        assert_eq!(result[3].severity_text, "INFO");
    }
    #[test]
    #[serial]
    fn test_map_logs_not_invocation_end() {
        // Set AWS_LAMBDA_EXEC_WRAPPER so is_auto_instrumented_disabled() returns false
        // This is required for the "put back to store" branch to be taken
        std::env::set_var("AWS_LAMBDA_EXEC_WRAPPER", "/opt/wrapper");
        std::env::remove_var("DASH0_DISABLE_AUTO_INSTRUMENTATION");

        let logs = vec![TelemetryLog {
            time: "2023-10-26T12:00:00.000Z".to_string(),
            r#type: "function".to_string(),
            record: json!("Log message"),
            invocation_id: Some("inv-not-end".to_string()),
        }];

        // When is_invocation_end is false, and no trace/span ID is stored,
        // it should put the log back to store and return empty list (retry later).
        let result = map_logs_to_otlp(&logs, false);

        assert_eq!(result.len(), 0);

        std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
    }
}

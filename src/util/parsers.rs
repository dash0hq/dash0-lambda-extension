/// Extract invocation id from Lambda Runtime API paths like
/// /<apiver>/runtime/invocation/<id>/response or /error
pub fn extract_invocation_id_from_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 6 && parts[2] == "runtime" && parts[3] == "invocation" {
        return Some(parts[4].to_string());
    }
    None
}

/// Extract invocation id from a Span's attributes (faas.invocation_id)
pub fn extract_invocation_id(span: &opentelemetry_proto::tonic::trace::v1::Span) -> Option<String> {
    span.attributes.iter().find_map(|attr| {
        if attr.key == "faas.invocation_id" {
            if let Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                value: Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(val)),
            }) = &attr.value
            {
                return Some(val.clone());
            }
        }
        None
    })
}

/// Extract invocation IDs for errored runtimeDone telemetry events from a payload
pub fn extract_error_invocation_ids(body_bytes: &[u8], body_text: &str) -> Vec<(String, String)> {
    use serde_json::Value as JsonValue;

    let extract_invocation_id = |event: &JsonValue| -> Option<(String, String)> {
        if event
            .get("type")
            .and_then(|t| t.as_str())
            .map(|t| t == "platform.runtimeDone")
            != Some(true)
        {
            return None;
        }

        let record = match event.get("record") {
            Some(r) => r,
            None => return None,
        };

        let is_error = matches!(
            record.get("status").and_then(|s| s.as_str()),
            Some("error") | Some("timeout")
        ) || record.get("errorType").is_some();

        if !is_error {
            return None;
        }

        let status = record.get("status").and_then(|s| s.as_str());
        let error_type = match status {
            Some("timeout") => Some("timeout".to_string()),
            Some("error") => record
                .get("errorType")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some("error".to_string())),
            _ => None,
        };

        let request_id = record
            .get("requestId")
            .and_then(|id| id.as_str())
            .map(|id| id.to_string());

        match (request_id, error_type) {
            (Some(id), Some(err_type)) => Some((id, err_type)),
            _ => None,
        }
    };

    match serde_json::from_slice::<JsonValue>(body_bytes) {
        Ok(json) => match &json {
            JsonValue::Array(events) => events.iter().filter_map(extract_invocation_id).collect(),
            _ => extract_invocation_id(&json).into_iter().collect(),
        },
        Err(_) => body_text
            .lines()
            .filter_map(|line| serde_json::from_str::<JsonValue>(line).ok())
            .filter_map(|event| extract_invocation_id(&event))
            .collect(),
    }
}

pub fn parse_otlp_endpoint() -> Option<(String, String)> {
    let lumigo_endpoint = match std::env::var("DASH0_ENDPOINT") {
        Ok(val) => val,
        Err(err) => {
            tracing::warn!(
                "[{}] endpoint not set; cannot send traces: {}",
                crate::log_prefix(),
                err
            );
            return None;
        }
    };

    let base_uri: hyper::Uri = match lumigo_endpoint.parse() {
        Ok(uri) => uri,
        Err(err) => {
            tracing::error!(
                "[{}] Invalid endpoint; cannot send traces: {}",
                crate::log_prefix(),
                err
            );
            return None;
        }
    };

    let scheme = match base_uri.scheme_str() {
        Some(s) => s.to_string(),
        None => {
            tracing::error!(
                "[{}] endpoint missing scheme; cannot send traces",
                crate::log_prefix(),
            );
            return None;
        }
    };

    let authority = match base_uri.authority() {
        Some(a) => a.to_string(),
        None => {
            tracing::error!(
                "[{}] endpoint missing authority; cannot send traces",
                crate::log_prefix(),
            );
            return None;
        }
    };

    Some((scheme, authority))
}

/// Generate a deterministic 16-byte trace ID from an invocation ID using SHA-256 hashing.
/// The same invocation ID will always produce the same trace ID.
pub fn get_trace_id_from_invocation_id(invocation_id: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"trace:");
    hasher.update(invocation_id.as_bytes());
    let hash = hasher.finalize();
    // Take first 16 bytes of the hash for the trace ID
    hash[..16].to_vec()
}

/// Generate a deterministic 8-byte span ID from an invocation ID using SHA-256 hashing.
/// The same invocation ID will always produce the same span ID.
pub fn get_span_id_from_invocation_id(invocation_id: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"span:");
    hasher.update(invocation_id.as_bytes());
    let hash = hasher.finalize();
    // Take first 8 bytes of the hash for the span ID
    hash[..8].to_vec()
}

/// Get the name of the instrumentation scope.
pub fn get_span_scope_name(
    request: &opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest,
) -> Option<String> {
    for resource_span in &request.resource_spans {
        for scope_span in &resource_span.scope_spans {
            if let Some(scope) = &scope_span.scope {
                return Some(scope.name.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        extract_error_invocation_ids, extract_invocation_id, extract_invocation_id_from_path,
        get_span_id_from_invocation_id, get_span_scope_name, get_trace_id_from_invocation_id,
        parse_otlp_endpoint,
    };
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use serial_test::serial;
    use std::env;

    #[test]
    fn test_get_span_scope_name() {
        let span = Span {
            name: "test-span-name".to_string(),
            ..Default::default()
        };
        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    scope: Some(
                        opentelemetry_proto::tonic::common::v1::InstrumentationScope {
                            name: "test-scope-name".to_string(),
                            ..Default::default()
                        },
                    ),
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        assert_eq!(
            get_span_scope_name(&request),
            Some("test-scope-name".to_string())
        );
    }

    // Tests for extract_invocation_id_from_path
    // ============================================================================

    #[test]
    fn test_extract_invocation_id_from_path_valid_response() {
        let path = "/2018-06-01/runtime/invocation/abc-123-def/response";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, Some("abc-123-def".to_string()));
    }

    #[test]
    fn test_extract_invocation_id_from_path_valid_error() {
        let path = "/2018-06-01/runtime/invocation/xyz-456-uvw/error";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, Some("xyz-456-uvw".to_string()));
    }

    #[test]
    fn test_extract_invocation_id_from_path_with_uuid() {
        let path = "/2018-06-01/runtime/invocation/550e8400-e29b-41d4-a716-446655440000/response";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(
            result,
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn test_extract_invocation_id_from_path_invalid_too_short() {
        let path = "/2018-06-01/runtime/invocation";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_invocation_id_from_path_invalid_wrong_structure() {
        let path = "/2018-06-01/wrong/path/abc-123/response";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_invocation_id_from_path_empty() {
        let path = "";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_invocation_id_from_path_root() {
        let path = "/";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_invocation_id_from_path_trailing_slash() {
        let path = "/2018-06-01/runtime/invocation/abc-123/response/";
        let result = extract_invocation_id_from_path(path);
        assert_eq!(result, Some("abc-123".to_string()));
    }

    #[test]
    fn test_extract_invocation_id_from_span_with_valid_attribute() {
        let span = Span {
            attributes: vec![KeyValue {
                key: "faas.invocation_id".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue("test-invocation-123".to_string())),
                }),
            }],
            ..Default::default()
        };
        assert_eq!(
            extract_invocation_id(&span),
            Some("test-invocation-123".to_string())
        );
    }

    #[test]
    fn test_extract_invocation_id_from_span_no_attributes() {
        let span = Span {
            attributes: vec![],
            ..Default::default()
        };
        assert_eq!(extract_invocation_id(&span), None);
    }

    #[test]
    fn test_extract_invocation_id_from_span_wrong_attribute() {
        let span = Span {
            attributes: vec![KeyValue {
                key: "other".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue("val".to_string())),
                }),
            }],
            ..Default::default()
        };
        assert_eq!(extract_invocation_id(&span), None);
    }

    #[test]
    fn test_extract_invocation_id_from_span_multiple_attributes() {
        let span = Span {
            attributes: vec![
                KeyValue {
                    key: "other".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue("val".to_string())),
                    }),
                },
                KeyValue {
                    key: "faas.invocation_id".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue("abc".to_string())),
                    }),
                },
            ],
            ..Default::default()
        };
        assert_eq!(extract_invocation_id(&span), Some("abc".to_string()));
    }

    #[test]
    fn test_extract_invocation_id_from_span_wrong_value_type() {
        let span = Span {
            attributes: vec![KeyValue {
                key: "faas.invocation_id".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::IntValue(5)),
                }),
            }],
            ..Default::default()
        };
        assert_eq!(extract_invocation_id(&span), None);
    }

    #[test]
    fn test_extract_error_invocation_ids_from_array_payload() {
        let payload = r#"[{"type":"platform.runtimeDone","record":{"status":"error","requestId":"inv-1"}},{"type":"platform.runtimeDone","record":{"status":"ok","requestId":"inv-2"}}]"#;
        let body_bytes = payload.as_bytes();
        let body_text = String::from_utf8_lossy(body_bytes);

        let result = extract_error_invocation_ids(body_bytes, &body_text);

        assert_eq!(result, vec![("inv-1".to_string(), "error".to_string())]);
    }

    #[test]
    fn test_extract_error_invocation_ids_from_newline_payload() {
        let payload = r#"{"type":"platform.runtimeDone","record":{"status":"timeout","requestId":"inv-3"}}
{"type":"platform.report","record":{"status":"ok","requestId":"inv-4"}}"#;
        let body_bytes = payload.as_bytes();
        let body_text = String::from_utf8_lossy(body_bytes);

        let result = extract_error_invocation_ids(body_bytes, &body_text);

        assert_eq!(result, vec![("inv-3".to_string(), "timeout".to_string())]);
    }

    #[test]
    fn test_extract_error_invocation_ids_ignores_non_runtime_done() {
        let payload =
            r#"{"type":"platform.report","record":{"status":"error","requestId":"inv-5"}}"#;
        let body_bytes = payload.as_bytes();
        let body_text = String::from_utf8_lossy(body_bytes);

        let result = extract_error_invocation_ids(body_bytes, &body_text);

        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_error_invocation_ids_handles_unrelated_json() {
        let payload = r#"{"foo": "bar", "nested": {"a": 1}}"#;
        let body_bytes = payload.as_bytes();
        let body_text = String::from_utf8_lossy(body_bytes);

        let result = extract_error_invocation_ids(body_bytes, &body_text);

        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_error_invocation_ids_uses_error_type_when_present() {
        let payload = r#"{"type":"platform.runtimeDone","record":{"status":"error","requestId":"inv-6","errorType":"CustomError"}}"#;
        let body_bytes = payload.as_bytes();
        let body_text = String::from_utf8_lossy(body_bytes);

        let result = extract_error_invocation_ids(body_bytes, &body_text);

        assert_eq!(
            result,
            vec![("inv-6".to_string(), "CustomError".to_string())]
        );
    }

    #[test]
    #[serial]
    fn test_parse_otlp_endpoint_success() {
        env::set_var("DASH0_ENDPOINT", "https://example.com/v1/traces");
        let result = parse_otlp_endpoint();
        assert_eq!(
            result,
            Some(("https".to_string(), "example.com".to_string()))
        );
    }

    #[test]
    #[serial]
    fn test_parse_otlp_endpoint_invalid() {
        env::set_var("DASH0_ENDPOINT", "example.com/v1/traces");
        let result = parse_otlp_endpoint();
        assert!(result.is_none());
    }

    #[test]
    #[serial]
    fn test_parse_otlp_endpoint_missing() {
        env::remove_var("DASH0_ENDPOINT");
        let result = parse_otlp_endpoint();
        assert!(result.is_none());
    }

    // Tests for get_trace_id_from_invocation_id
    // ============================================================================

    #[test]
    fn test_get_trace_id_from_invocation_id_deterministic() {
        let invocation_id = "test-inv-123";
        let trace_id_1 = get_trace_id_from_invocation_id(invocation_id);
        let trace_id_2 = get_trace_id_from_invocation_id(invocation_id);
        let trace_id_3 = get_trace_id_from_invocation_id(invocation_id);

        assert_eq!(trace_id_1, trace_id_2);
        assert_eq!(trace_id_2, trace_id_3);
        assert_eq!(trace_id_1.len(), 16);
    }

    #[test]
    fn test_get_trace_id_from_invocation_id_unique() {
        let trace_id_1 = get_trace_id_from_invocation_id("inv-1");
        let trace_id_2 = get_trace_id_from_invocation_id("inv-2");
        let trace_id_3 = get_trace_id_from_invocation_id("inv-3");

        assert_ne!(trace_id_1, trace_id_2);
        assert_ne!(trace_id_2, trace_id_3);
        assert_ne!(trace_id_1, trace_id_3);
    }

    #[test]
    fn test_get_trace_id_from_invocation_id_length() {
        let trace_id = get_trace_id_from_invocation_id("any-invocation-id");
        assert_eq!(trace_id.len(), 16);
    }

    // Tests for get_span_id_from_invocation_id
    // ============================================================================

    #[test]
    fn test_get_span_id_from_invocation_id_deterministic() {
        let invocation_id = "test-inv-456";
        let span_id_1 = get_span_id_from_invocation_id(invocation_id);
        let span_id_2 = get_span_id_from_invocation_id(invocation_id);
        let span_id_3 = get_span_id_from_invocation_id(invocation_id);

        assert_eq!(span_id_1, span_id_2);
        assert_eq!(span_id_2, span_id_3);
        assert_eq!(span_id_1.len(), 8);
    }

    #[test]
    fn test_get_span_id_from_invocation_id_unique() {
        let span_id_1 = get_span_id_from_invocation_id("inv-1");
        let span_id_2 = get_span_id_from_invocation_id("inv-2");
        let span_id_3 = get_span_id_from_invocation_id("inv-3");

        assert_ne!(span_id_1, span_id_2);
        assert_ne!(span_id_2, span_id_3);
        assert_ne!(span_id_1, span_id_3);
    }

    #[test]
    fn test_get_span_id_from_invocation_id_length() {
        let span_id = get_span_id_from_invocation_id("any-invocation-id");
        assert_eq!(span_id.len(), 8);
    }

    #[test]
    fn test_trace_id_and_span_id_are_different() {
        let invocation_id = "same-invocation-id";
        let trace_id = get_trace_id_from_invocation_id(invocation_id);
        let span_id = get_span_id_from_invocation_id(invocation_id);

        // Even though they're from the same invocation ID, they should be different
        // (comparing first 8 bytes of trace_id with span_id)
        assert_ne!(&trace_id[..8], &span_id[..]);
    }
}

use crate::store::{
    get_event_payload, store_return_payload, store_traces, take_return_payload, take_traces,
    StoredTrace,
};
use crate::util::log_mutations::try_read_env_from_file;
use crate::util::parsers::{
    extract_invocation_id, get_span_id_from_invocation_id, get_span_scope_name,
    get_trace_id_from_invocation_id,
};
use hyper::header;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::span::{Event, SpanKind};
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, Status};
use prost::Message;
use serde_json::Map as JsonMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn drop_duplicate_java_instrumenations(decoded: &ExportTraceServiceRequest) -> bool {
    let scope_name = get_span_scope_name(decoded);
    scope_name.as_deref() == Some("io.opentelemetry.aws-lambda-core-1.0")
}

pub fn build_runtime_error_trace(
    invocation_id: &str,
    error_type: Option<&str>,
    return_value: Option<&str>,
    existing_traces: &[StoredTrace],
) -> Option<StoredTrace> {
    let (trace_id, span_id) = get_trace_span_ids(invocation_id, existing_traces);

    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let start_nanos = crate::store::get_invocation_data(invocation_id)
        .map(|data| (data.start_time * 1_000_000.0) as u64)
        .filter(|&t| t > 0)
        .unwrap_or(now_nanos);

    let mut attributes = vec![
        KeyValue {
            key: "faas.invocation_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(invocation_id.to_string())),
            }),
        },
        KeyValue {
            key: "cloud.resource_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_function_arn().unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
        KeyValue {
            key: "cloud.account.id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_account_id().unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
    ];

    if let Some(event_payload) = get_event_payload(invocation_id) {
        attributes.push(KeyValue {
            key: "faas.event".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(event_payload)),
            }),
        });
    } else {
        tracing::warn!(
            "[{}] No stored event payload found for invocation id {}",
            crate::log_prefix(),
            invocation_id
        );
    }

    if let Some(ret) = return_value {
        attributes.push(KeyValue {
            key: "faas.return_value".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(ret.to_string())),
            }),
        });
    }

    let exception_result = create_exception_event(error_type, return_value, now_nanos);
    let (events, status) = match exception_result {
        Some((event, err_type)) => (
            vec![event],
            Status {
                code: StatusCode::Error as i32,
                message: err_type,
                ..Default::default()
            },
        ),
        None => (vec![], Default::default()),
    };

    let span = Span {
        trace_id,
        span_id,
        name: "unknown".to_string(),
        kind: SpanKind::Server as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: now_nanos,
        attributes,
        events,
        status: Some(status),
        ..Default::default()
    };

    let scope_spans = ScopeSpans {
        scope: Some(InstrumentationScope {
            name: "opentelemetry.instrumentation.aws_lambda".to_string(),
            version: "unknown".to_string(),
            ..Default::default()
        }),
        spans: vec![span],
        schema_url: "https://opentelemetry.io/schemas/1.11.0".to_string(),
    };

    let resource = Resource {
        attributes: vec![
            KeyValue {
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
            KeyValue {
                key: "process.environ".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(env_as_json_string())),
                }),
            },
        ],
        ..Default::default()
    };

    let export = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(resource),
            scope_spans: vec![scope_spans],
            ..Default::default()
        }],
    };

    let mut headers = hyper::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/x-protobuf"),
    );

    Some(StoredTrace {
        method: hyper::Method::POST,
        path_and_query: "/v1/traces".to_string(),
        headers,
        body: export.encode_to_vec(),
        invocation_ids: vec![invocation_id.to_string()],
    })
}

fn create_exception_event(
    error_type: Option<&str>,
    return_value: Option<&str>,
    now_nanos: u64,
) -> Option<(Event, String)> {
    if let Some(err) = error_type {
        return Some((
            Event {
                time_unix_nano: now_nanos,
                name: "exception".to_string(),
                attributes: vec![
                    KeyValue {
                        key: "exception.type".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(err.to_string())),
                        }),
                    },
                    KeyValue {
                        key: "exception.message".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(err.to_string())),
                        }),
                    },
                    KeyValue {
                        key: "exception.escaped".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue("False".to_string())),
                        }),
                    },
                ],
                ..Default::default()
            },
            err.to_string(),
        ));
    }

    if let Some(ret) = return_value {
        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(ret) {
            let error_message = json_val.get("errorMessage").and_then(|v| v.as_str());
            let error_type = json_val.get("errorType").and_then(|v| v.as_str());

            if let (Some(msg), Some(typ)) = (error_message, error_type) {
                let mut attributes = vec![
                    KeyValue {
                        key: "exception.type".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(typ.to_string())),
                        }),
                    },
                    KeyValue {
                        key: "exception.message".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(msg.to_string())),
                        }),
                    },
                    KeyValue {
                        key: "exception.escaped".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue("False".to_string())),
                        }),
                    },
                ];

                if let Some(stack_trace) = json_val.get("stackTrace").and_then(|v| v.as_array()) {
                    let stack_trace_str = stack_trace
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<&str>>()
                        .join("");
                    attributes.push(KeyValue {
                        key: "exception.stacktrace".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(stack_trace_str)),
                        }),
                    });
                }

                return Some((
                    Event {
                        time_unix_nano: now_nanos,
                        name: "exception".to_string(),
                        attributes: attributes,
                        ..Default::default()
                    },
                    typ.to_string(),
                ));
            }

            if let Some(status_code) = json_val.get("statusCode").and_then(|v| v.as_i64()) {
                if status_code >= 400 {
                    let body = json_val
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or("500");
                    return Some((
                        Event {
                            time_unix_nano: now_nanos,
                            name: "exception".to_string(),
                            attributes: vec![
                                KeyValue {
                                    key: "exception.type".to_string(),
                                    value: Some(AnyValue {
                                        value: Some(Value::StringValue(status_code.to_string())),
                                    }),
                                },
                                KeyValue {
                                    key: "exception.message".to_string(),
                                    value: Some(AnyValue {
                                        value: Some(Value::StringValue(body.to_string())),
                                    }),
                                },
                                KeyValue {
                                    key: "exception.escaped".to_string(),
                                    value: Some(AnyValue {
                                        value: Some(Value::StringValue("False".to_string())),
                                    }),
                                },
                            ],
                            ..Default::default()
                        },
                        status_code.to_string(),
                    ));
                }
            }
        }
    }

    None
}

fn is_lambda_instrumentation_scope(scope_name: &str) -> bool {
    scope_name == "opentelemetry.instrumentation.aws_lambda"
        || scope_name == "@opentelemetry/instrumentation-aws-lambda"
        || scope_name == "io.opentelemetry.aws-lambda-core-1.0"
        || scope_name == "io.opentelemetry.aws-lambda-events-2.2"
        || scope_name == "OpenTelemetry.Instrumentation.AWSLambda"
}

pub fn add_event_payload_to_lambda_server_spans(
    request: &mut ExportTraceServiceRequest,
    invocation_ids: &mut Vec<String>,
) -> bool {
    let mut added = false;
    for resource_span in &mut request.resource_spans {
        for scope_span in &mut resource_span.scope_spans {
            if let Some(scope) = &scope_span.scope {
                if is_lambda_instrumentation_scope(&scope.name) {
                    added |= annotate_server_spans(&mut scope_span.spans, invocation_ids);
                }
            }
        }
    }
    added
}

fn annotate_server_spans(spans: &mut Vec<Span>, invocation_ids: &mut Vec<String>) -> bool {
    let mut touched = false;
    for span in spans {
        if let Some(invocation_id) = extract_invocation_id(span) {
            invocation_ids.push(invocation_id.clone());

            if let Some(event_payload) = get_event_payload(&invocation_id) {
                span.attributes.push(KeyValue {
                    key: "faas.event".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue(event_payload)),
                    }),
                });
                touched = true;
            } else {
                tracing::warn!(
                    "[{}] No stored event payload found for invocation id {}",
                    crate::log_prefix(),
                    invocation_id
                );
            }
        }
    }
    touched
}

pub fn add_return_payload_to_lambda_server_spans(
    invocation_id: &str,
    return_payload: &str,
) -> bool {
    let mut traces = take_traces();
    let mut updated_traces: Vec<StoredTrace> = Vec::new();
    let mut added = false;

    for mut trace in traces.drain(..) {
        let mut modified = false;
        match ExportTraceServiceRequest::decode(trace.body.as_slice()) {
            Ok(mut decoded) => {
                modified = annotate_return_payload(&mut decoded, invocation_id, return_payload)
                    || modified;

                if modified {
                    trace.body = decoded.encode_to_vec();
                    added = true;
                }
            }
            Err(err) => {
                tracing::error!(
                    "[{}] Failed to decode trace payload while adding return value for {}: {}",
                    crate::log_prefix(),
                    invocation_id,
                    err
                );
            }
        }

        updated_traces.push(trace);
    }

    store_traces(updated_traces);
    if added {
        // Clean up any pending payload stored earlier for this invocation.
        let _ = take_return_payload(invocation_id);
    } else {
        store_return_payload(invocation_id, return_payload);
    }
    added
}

pub fn annotate_return_payload(
    request: &mut ExportTraceServiceRequest,
    invocation_id: &str,
    return_payload: &str,
) -> bool {
    let mut modified = false;
    for resource_span in &mut request.resource_spans {
        for scope_span in &mut resource_span.scope_spans {
            if let Some(scope) = &scope_span.scope {
                if is_lambda_instrumentation_scope(&scope.name) {
                    for span in &mut scope_span.spans {
                        if let Some(id) = extract_invocation_id(span) {
                            if id == invocation_id {
                                span.attributes.push(KeyValue {
                                    key: "faas.return_value".to_string(),
                                    value: Some(AnyValue {
                                        value: Some(Value::StringValue(return_payload.to_string())),
                                    }),
                                });
                                modified = true;
                            }
                        }
                    }
                }
            }
        }
    }
    modified
}

pub fn merge_telemetry_invocation_data(request: &mut ExportTraceServiceRequest) -> i32 {
    let mut modified = 0;
    for resource_span in &mut request.resource_spans {
        for scope_span in &mut resource_span.scope_spans {
            if let Some(scope) = &scope_span.scope {
                if is_lambda_instrumentation_scope(&scope.name) {
                    for span in &mut scope_span.spans {
                        if let Some(invocation_id) = extract_invocation_id(span) {
                            if let Some(data) = crate::store::get_invocation_data(&invocation_id) {
                                if data.init_duration > 0.0 {
                                    span.attributes.push(KeyValue {
                                        key: "faas.init_duration".to_string(),
                                        value: Some(AnyValue {
                                            value: Some(Value::DoubleValue(data.init_duration)),
                                        }),
                                    });
                                    modified += 1;
                                }
                                if data.billed_duration > 0.0 {
                                    span.attributes.push(KeyValue {
                                        key: "faas.billed_duration".to_string(),
                                        value: Some(AnyValue {
                                            value: Some(Value::DoubleValue(data.billed_duration)),
                                        }),
                                    });
                                    modified += 1;
                                }
                                if data.memory_usage > 0 {
                                    span.attributes.push(KeyValue {
                                        key: "faas.memory_used".to_string(),
                                        value: Some(AnyValue {
                                            value: Some(Value::IntValue(data.memory_usage as i64)),
                                        }),
                                    });
                                    modified += 1;
                                }

                                if data.start_time > 0.0 {
                                    span.start_time_unix_nano =
                                        (data.start_time * 1_000_000.0) as u64;
                                    modified += 1;
                                }
                                if data.end_time > 0.0 {
                                    span.end_time_unix_nano = (data.end_time * 1_000_000.0) as u64;
                                    modified += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    modified
}

/// Process a decoded trace request by adding event payloads, return payloads, and storing invocation span IDs.
/// Returns true if the request was modified and needs to be re-encoded.
pub fn process_trace_request(
    decoded: &mut ExportTraceServiceRequest,
    invocation_ids: &mut Vec<String>,
    encoded_body: &mut Vec<u8>,
) -> bool {
    let mut modified = false;

    // Add event payload to lambda server spans
    let added = add_event_payload_to_lambda_server_spans(decoded, invocation_ids);
    if added {
        *encoded_body = decoded.encode_to_vec();
        tracing::info!(
            "[{}] /v1/traces added faas.event payload to lambda server span. invocation_ids={:?}",
            crate::log_prefix(),
            invocation_ids
        );
        modified = true;
    } else {
        tracing::info!(
            "[{}] /v1/traces no lambda server span found to annotate",
            crate::log_prefix()
        );
    }

    match serde_json::to_string(&decoded) {
        Ok(json) => tracing::trace!(
            "[{}] /v1/traces forward payload (json): {}",
            crate::log_prefix(),
            json
        ),
        Err(err) => tracing::error!(
            "[{}] /v1/traces failed to render json: {}",
            crate::log_prefix(),
            err
        ),
    }

    // If we have pending return payloads for these invocation IDs, apply them now.
    let mut updated_with_return = false;
    for id in invocation_ids.iter() {
        if let Some(payload) = crate::store::take_return_payload(id) {
            if annotate_return_payload(decoded, id, &payload) {
                updated_with_return = true;
            }
        }
    }
    if updated_with_return {
        *encoded_body = decoded.encode_to_vec();
        tracing::info!(
            "[{}] /v1/traces added pending faas.return_value to lambda server span. invocation_ids={:?}", crate::log_prefix(),
            invocation_ids
        );
        modified = true;
    }

    // Check for faas.invocation_id and store trace/span id mapping
    for resource_span in &decoded.resource_spans {
        for scope_span in &resource_span.scope_spans {
            for span in &scope_span.spans {
                for attribute in &span.attributes {
                    if attribute.key == "faas.invocation_id" {
                        if let Some(val) = &attribute.value {
                            if let Some(Value::StringValue(invocation_id)) = &val.value {
                                let trace_id_hex = span
                                    .trace_id
                                    .iter()
                                    .map(|b| format!("{:02x}", b))
                                    .collect::<String>();
                                let span_id_hex = span
                                    .span_id
                                    .iter()
                                    .map(|b| format!("{:02x}", b))
                                    .collect::<String>();
                                crate::store::store_invocation_span_id(
                                    invocation_id,
                                    trace_id_hex,
                                    span_id_hex,
                                );
                                tracing::debug!(
                                    "[{}] stored trace/span id for invocation_id={}",
                                    crate::log_prefix(),
                                    invocation_id
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    modified
}

#[cfg(test)]
mod tests {
    use super::{
        add_event_payload_to_lambda_server_spans, add_return_payload_to_lambda_server_spans,
        annotate_return_payload, build_runtime_error_trace, StatusCode,
    };
    use crate::store::{
        snapshot_traces, store_event_payload, store_return_payload, store_trace,
        take_return_payload, take_traces, StoredTrace,
    };
    use hyper::{header, Method};
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{span::SpanKind, ResourceSpans, ScopeSpans, Span};
    use prost::Message;
    use serial_test::serial;

    fn find_attribute<'a>(span: &'a Span, key: &'a str) -> Option<&'a AnyValue> {
        span.attributes
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
    }

    #[test]
    #[serial]
    fn builds_trace_with_event_payload_and_invocation_id() {
        let invocation_id = "inv-test-1";
        store_event_payload(invocation_id, r#"{"foo":"bar"}"#);

        let trace = build_runtime_error_trace(invocation_id, Some("CustomError"), None, &[])
            .expect("trace should build");

        assert_eq!(trace.method, Method::POST);
        assert_eq!(trace.path_and_query, "/v1/traces");
        assert_eq!(trace.invocation_ids, vec![invocation_id.to_string()]);
        assert_eq!(
            trace
                .headers
                .get(header::CONTENT_TYPE)
                .map(|v| v.to_str().unwrap()),
            Some("application/x-protobuf")
        );

        let decoded = ExportTraceServiceRequest::decode(trace.body.as_slice())
            .expect("should decode otlp payload");
        let span = decoded.resource_spans[0].scope_spans[0].spans[0].clone();

        let invocation_attr = find_attribute(&span, "faas.invocation_id");
        assert_eq!(
            invocation_attr
                .and_then(|v| v.value.as_ref())
                .and_then(|v| match v {
                    Value::StringValue(s) => Some(s),
                    _ => None,
                }),
            Some(&invocation_id.to_string())
        );

        let event_attr = find_attribute(&span, "faas.event");
        assert!(event_attr.is_some(), "faas.event should be included");

        let exception = span
            .events
            .iter()
            .find(|e| e.name == "exception")
            .cloned()
            .expect("exception event exists");
        assert_eq!(exception.attributes.len(), 3);
        let ex_type = exception
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(ex_type, Some("CustomError".to_string()));
        assert!(
            matches!(span.status.as_ref().map(|s| s.code), Some(c) if c == StatusCode::Error as i32)
        );
    }

    #[test]
    #[serial]
    fn builds_trace_without_event_payload() {
        let invocation_id = "inv-test-2";

        let trace = build_runtime_error_trace(invocation_id, Some("error"), None, &[])
            .expect("trace should build");

        let decoded = ExportTraceServiceRequest::decode(trace.body.as_slice())
            .expect("should decode otlp payload");
        let span = decoded.resource_spans[0].scope_spans[0].spans[0].clone();

        let event_attr = find_attribute(&span, "faas.event");
        assert!(
            event_attr.is_none(),
            "faas.event should be absent when not stored"
        );

        let exception = span
            .events
            .iter()
            .find(|e| e.name == "exception")
            .cloned()
            .expect("exception event exists");
        let ex_type = exception
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(ex_type, Some("error".to_string()));
    }

    fn make_span_with_invocation(invocation_id: &str) -> Span {
        Span {
            attributes: vec![KeyValue {
                key: "faas.invocation_id".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(invocation_id.to_string())),
                }),
            }],
            ..Default::default()
        }
    }

    fn make_request_with_scope(name: &str, span: Span) -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource::default()),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: name.to_string(),
                        ..Default::default()
                    }),
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    #[serial]
    fn add_event_payload_adds_attribute_and_invocation_id() {
        let invocation_id = "inv-event-1";
        store_event_payload(invocation_id, r#"{"hello":"world"}"#);
        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();

        let added = add_event_payload_to_lambda_server_spans(&mut request, &mut invocation_ids);

        assert!(added, "expected faas.event to be added");
        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let event_attr = find_attribute(span, "faas.event");
        assert!(event_attr.is_some(), "faas.event attribute should exist");
    }

    #[test]
    #[serial]
    fn add_event_payload_handles_missing_payload() {
        let invocation_id = "inv-event-2";
        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();

        let added = add_event_payload_to_lambda_server_spans(&mut request, &mut invocation_ids);

        assert!(!added, "no faas.event should be added without payload");
        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let event_attr = find_attribute(span, "faas.event");
        assert!(
            event_attr.is_none(),
            "faas.event attribute should be absent"
        );
    }

    #[test]
    fn add_event_payload_ignores_other_scopes() {
        let invocation_id = "inv-event-3";
        store_event_payload(invocation_id, r#"{"foo":"bar"}"#);
        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("other.scope", span);
        let mut invocation_ids = Vec::new();

        let added = add_event_payload_to_lambda_server_spans(&mut request, &mut invocation_ids);

        assert!(
            !added,
            "non-lambda scopes should not be annotated with payloads"
        );
        assert!(
            invocation_ids.is_empty(),
            "invocation_ids should remain empty for non-matching scopes"
        );
        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let event_attr = find_attribute(span, "faas.event");
        assert!(
            event_attr.is_none(),
            "faas.event attribute should be absent"
        );
    }

    #[test]
    #[serial]
    fn add_return_payload_adds_attribute_for_matching_server_span() {
        take_traces();
        let invocation_id = "inv-return-1";
        let mut span = make_span_with_invocation(invocation_id);
        span.kind = SpanKind::Server as i32;
        let request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let trace = StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: hyper::HeaderMap::new(),
            body: request.encode_to_vec(),
            invocation_ids: vec![invocation_id.to_string()],
        };
        store_trace(trace);

        add_return_payload_to_lambda_server_spans(invocation_id, "result");

        let traces = take_traces();
        assert_eq!(traces.len(), 1);
        let decoded = ExportTraceServiceRequest::decode(traces[0].body.as_slice())
            .expect("should decode updated trace");
        let span = &decoded.resource_spans[0].scope_spans[0].spans[0];
        let attr = find_attribute(span, "faas.return_value");
        assert!(attr.is_some(), "faas.return_value should be added");
    }

    #[test]
    #[serial]
    fn add_return_payload_ignores_non_matching_invocation() {
        take_traces();
        let mut span = make_span_with_invocation("other-inv");
        span.kind = SpanKind::Server as i32;
        let request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let trace = StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: hyper::HeaderMap::new(),
            body: request.encode_to_vec(),
            invocation_ids: vec!["other-inv".to_string()],
        };
        store_trace(trace);

        add_return_payload_to_lambda_server_spans("inv-return-2", "result");

        let traces = take_traces();
        assert_eq!(traces.len(), 1);
        let decoded = ExportTraceServiceRequest::decode(traces[0].body.as_slice())
            .expect("should decode updated trace");
        let span = &decoded.resource_spans[0].scope_spans[0].spans[0];
        let attr = find_attribute(span, "faas.return_value");
        assert!(
            attr.is_none(),
            "faas.return_value should not be added for non-matching invocation"
        );
    }

    #[test]
    #[serial]
    fn add_return_payload_stores_when_trace_not_found() {
        take_traces();
        let invocation_id = "inv-return-store";

        let added = add_return_payload_to_lambda_server_spans(invocation_id, "result");

        assert!(
            !added,
            "should not mark as added when no trace is available to annotate"
        );
        let stored = take_return_payload(invocation_id);
        assert_eq!(stored, Some("result".to_string()));
    }

    #[test]
    #[serial]
    fn annotate_return_payload_applies_pending_and_clears_store() {
        let invocation_id = "inv-return-late";
        store_return_payload(invocation_id, "late_result");
        let payload = take_return_payload(invocation_id).expect("payload should be stored");
        let mut span = make_span_with_invocation(invocation_id);
        span.kind = SpanKind::Server as i32;
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);

        let added = annotate_return_payload(&mut request, invocation_id, &payload);

        assert!(added, "pending return payload should be applied");
        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let attr = find_attribute(span, "faas.return_value");
        assert!(
            attr.is_some(),
            "faas.return_value attribute should be present after applying pending payload"
        );
        assert!(
            take_return_payload(invocation_id).is_none(),
            "pending payload should be cleared after applying"
        );
    }

    #[test]
    #[serial]
    fn build_runtime_error_trace_uses_existing_trace_and_parent_ids() {
        take_traces();
        let invocation_id = "inv-trace-copy";
        let trace_id = vec![1u8; 16];
        let parent_span_id = vec![2u8; 8];
        let mut span = make_span_with_invocation(invocation_id);
        span.trace_id = trace_id.clone();
        span.span_id = vec![3u8; 8];
        span.parent_span_id = parent_span_id.clone();

        let request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let trace = StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: hyper::HeaderMap::new(),
            body: request.encode_to_vec(),
            invocation_ids: vec![invocation_id.to_string()],
        };
        store_trace(trace);

        let traces = snapshot_traces();
        let synthetic =
            build_runtime_error_trace(invocation_id, Some("CopiedError"), None, &traces)
                .expect("trace should build");

        let decoded = ExportTraceServiceRequest::decode(synthetic.body.as_slice())
            .expect("decode synthetic trace");
        let span = &decoded.resource_spans[0].scope_spans[0].spans[0];
        assert_eq!(span.trace_id, trace_id);
        assert_eq!(span.span_id, parent_span_id);
    }

    #[test]
    #[serial]
    fn process_trace_request_adds_event_payload() {
        let invocation_id = "inv-process-1";
        store_event_payload(invocation_id, r#"{"test":"data"}"#);

        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        let modified =
            super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert!(
            modified,
            "request should be modified when event payload is added"
        );
        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        assert!(!encoded_body.is_empty(), "encoded_body should be updated");

        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let event_attr = find_attribute(span, "faas.event");
        assert!(event_attr.is_some(), "faas.event should be added");
    }

    #[test]
    #[serial]
    fn process_trace_request_applies_pending_return_payload() {
        let invocation_id = "inv-process-2";
        store_event_payload(invocation_id, r#"{"test":"data"}"#);
        store_return_payload(invocation_id, r#"{"result":"success"}"#);

        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        let modified =
            super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert!(
            modified,
            "request should be modified when return payload is added"
        );
        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);

        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let return_attr = find_attribute(span, "faas.return_value");
        assert!(return_attr.is_some(), "faas.return_value should be added");

        // Verify the pending payload was consumed
        assert!(
            take_return_payload(invocation_id).is_none(),
            "pending return payload should be consumed"
        );
    }

    #[test]
    #[serial]
    fn process_trace_request_stores_invocation_span_ids() {
        let invocation_id = "inv-process-3";
        let trace_id = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let span_id = vec![1u8, 2, 3, 4, 5, 6, 7, 8];

        let mut span = make_span_with_invocation(invocation_id);
        span.trace_id = trace_id.clone();
        span.span_id = span_id.clone();

        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        // Verify the span IDs were stored
        let stored = crate::store::get_invocation_span_id(invocation_id);
        assert!(stored.is_some(), "invocation span IDs should be stored");

        let stored = stored.unwrap();
        let expected_trace_id = trace_id
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        let expected_span_id = span_id
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

        assert_eq!(stored.trace_id, expected_trace_id);
        assert_eq!(stored.span_id, expected_span_id);
    }

    #[test]
    #[serial]
    fn process_trace_request_handles_multiple_invocation_ids() {
        let invocation_id_1 = "inv-process-4a";
        let invocation_id_2 = "inv-process-4b";

        store_event_payload(invocation_id_1, r#"{"test":"data1"}"#);
        store_event_payload(invocation_id_2, r#"{"test":"data2"}"#);
        store_return_payload(invocation_id_2, r#"{"result":"success2"}"#);

        let span1 = make_span_with_invocation(invocation_id_1);
        let span2 = make_span_with_invocation(invocation_id_2);

        let mut request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource::default()),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "opentelemetry.instrumentation.aws_lambda".to_string(),
                        ..Default::default()
                    }),
                    spans: vec![span1, span2],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        let modified =
            super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert!(modified, "request should be modified");
        assert_eq!(invocation_ids.len(), 2);
        assert!(invocation_ids.contains(&invocation_id_1.to_string()));
        assert!(invocation_ids.contains(&invocation_id_2.to_string()));

        // Verify both spans were annotated
        let spans = &request.resource_spans[0].scope_spans[0].spans;
        assert_eq!(spans.len(), 2);

        for span in spans {
            let event_attr = find_attribute(span, "faas.event");
            assert!(event_attr.is_some(), "both spans should have faas.event");
        }

        // Verify only the second span has return value
        let span2_return = find_attribute(&spans[1], "faas.return_value");
        assert!(
            span2_return.is_some(),
            "second span should have faas.return_value"
        );
    }

    #[test]
    #[serial]
    fn process_trace_request_returns_false_when_no_modifications() {
        let invocation_id = "inv-process-5";

        // Don't store any event payload or return payload
        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        let modified =
            super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert!(
            !modified,
            "request should not be modified when no payloads are available"
        );
        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        assert!(encoded_body.is_empty(), "encoded_body should remain empty");
    }

    #[test]
    #[serial]
    fn process_trace_request_ignores_non_lambda_scopes() {
        let invocation_id = "inv-process-6";
        store_event_payload(invocation_id, r#"{"test":"data"}"#);

        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("other.instrumentation.scope", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        let modified =
            super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert!(!modified, "non-lambda scopes should not be modified");
        assert!(
            invocation_ids.is_empty(),
            "invocation_ids should remain empty for non-lambda scopes"
        );
    }

    #[test]
    #[serial]
    fn process_trace_request_updates_encoded_body_correctly() {
        let invocation_id = "inv-process-7";
        store_event_payload(invocation_id, r#"{"test":"data"}"#);

        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        // Verify the encoded body can be decoded and matches the request
        let decoded = ExportTraceServiceRequest::decode(encoded_body.as_slice())
            .expect("encoded_body should be valid OTLP");

        assert_eq!(decoded.resource_spans.len(), request.resource_spans.len());
        let decoded_span = &decoded.resource_spans[0].scope_spans[0].spans[0];
        let request_span = &request.resource_spans[0].scope_spans[0].spans[0];

        assert_eq!(decoded_span.attributes.len(), request_span.attributes.len());
        let event_attr = find_attribute(decoded_span, "faas.event");
        assert!(event_attr.is_some(), "decoded span should have faas.event");
    }

    #[test]
    #[serial]
    fn builds_trace_with_error_from_return_value() {
        let invocation_id = "inv-error-json";
        let error_json = r#"{"errorMessage": "Something went wrong", "errorType": "ValueError", "requestId": "123", "stackTrace": ["line1\n", "line2\n"]}"#;

        let trace = build_runtime_error_trace(invocation_id, None, Some(error_json), &[])
            .expect("trace should build");

        let decoded = ExportTraceServiceRequest::decode(trace.body.as_slice())
            .expect("should decode otlp payload");
        let span = decoded.resource_spans[0].scope_spans[0].spans[0].clone();

        let exception = span
            .events
            .iter()
            .find(|e| e.name == "exception")
            .cloned()
            .expect("exception event exists");

        let ex_type = exception
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(ex_type, Some("ValueError".to_string()));

        let ex_message = exception
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.message")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(ex_message, Some("Something went wrong".to_string()));

        let ex_stack = exception
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.stacktrace")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(ex_stack, Some("line1\nline2\n".to_string()));
    }

    #[test]
    fn create_exception_event_with_error_type() {
        let error_type = Some("MyError");
        let return_value = None;
        let now = 123456789;

        let (event, err_type) = super::create_exception_event(error_type, return_value, now)
            .expect("should create event");

        assert_eq!(err_type, "MyError");

        assert_eq!(event.name, "exception");
        assert_eq!(event.time_unix_nano, now);

        let type_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(type_attr, Some("MyError".to_string()));
    }

    #[test]
    fn create_exception_event_from_return_value() {
        let error_type = None;
        let return_value = Some(
            r#"{"errorMessage": "ReturnError", "errorType": "ReturnType", "stackTrace": ["stack1", "stack2"]}"#,
        );
        let now = 987654321;

        let (event, err_type) = super::create_exception_event(error_type, return_value, now)
            .expect("should create event");

        assert_eq!(err_type, "ReturnType");

        assert_eq!(event.name, "exception");
        assert_eq!(event.time_unix_nano, now);

        let type_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(type_attr, Some("ReturnType".to_string()));

        let msg_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.message")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(msg_attr, Some("ReturnError".to_string()));

        let stack_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.stacktrace")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(stack_attr, Some("stack1stack2".to_string()));
    }

    #[test]
    fn create_exception_event_from_status_code() {
        let error_type = None;
        let return_value = Some(r#"{"statusCode": 500, "body": "Internal Server Error"}"#);
        let now = 1122334455;

        let (event, err_type) = super::create_exception_event(error_type, return_value, now)
            .expect("should create event");

        assert_eq!(err_type, "500");
        assert_eq!(event.name, "exception");
        assert_eq!(event.time_unix_nano, now);

        let type_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(type_attr, Some("500".to_string()));

        let msg_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.message")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(msg_attr, Some("Internal Server Error".to_string()));

        let escaped_attr = event
            .attributes
            .iter()
            .find(|kv| kv.key == "exception.escaped")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(escaped_attr, Some("False".to_string()));
    }

    #[test]
    #[serial]
    fn add_event_payload_support_nodejs_scope() {
        let invocation_id = "inv-event-node";
        store_event_payload(invocation_id, r#"{"hello":"node"}"#);
        let span = make_span_with_invocation(invocation_id);
        let mut request =
            make_request_with_scope("@opentelemetry/instrumentation-aws-lambda", span);
        let mut invocation_ids = Vec::new();

        let added = add_event_payload_to_lambda_server_spans(&mut request, &mut invocation_ids);

        assert!(added, "expected faas.event to be added for nodejs scope");
        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        let event_attr = find_attribute(span, "faas.event");
        assert!(event_attr.is_some(), "faas.event attribute should exist");
    }

    #[test]
    #[serial]
    fn add_return_payload_support_nodejs_scope() {
        take_traces();
        let invocation_id = "inv-return-node";
        let mut span = make_span_with_invocation(invocation_id);
        span.kind = SpanKind::Server as i32;
        let request = make_request_with_scope("@opentelemetry/instrumentation-aws-lambda", span);
        let trace = StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: hyper::HeaderMap::new(),
            body: request.encode_to_vec(),
            invocation_ids: vec![invocation_id.to_string()],
        };
        store_trace(trace);

        add_return_payload_to_lambda_server_spans(invocation_id, "node_result");

        let traces = take_traces();
        assert_eq!(traces.len(), 1);
        let decoded = ExportTraceServiceRequest::decode(traces[0].body.as_slice())
            .expect("should decode updated trace");
        let span = &decoded.resource_spans[0].scope_spans[0].spans[0];
        let attr = find_attribute(span, "faas.return_value");
        assert!(
            attr.is_some(),
            "faas.return_value should be added for nodejs scope"
        );
    }
    #[test]
    #[serial]
    fn test_merge_telemetry_invocation_data_updates_span() {
        let invocation_id = "inv-merge-data";

        // Setup InvocationData
        crate::store::update_invocation_data(invocation_id, |data| {
            data.init_duration = 100.0;
            data.billed_duration = 200.0;
            data.memory_usage = 128;
            data.start_time = 1_000.0; // 1 second
            data.end_time = 2_000.0; // 2 seconds
        });

        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);

        let modified = super::merge_telemetry_invocation_data(&mut request);

        assert!(modified > 0, "request should be modified");

        let span = &request.resource_spans[0].scope_spans[0].spans[0];

        // attributes
        let init_attr = find_attribute(span, "faas.init_duration").and_then(|v| match &v.value {
            Some(Value::DoubleValue(d)) => Some(*d),
            _ => None,
        });
        assert_eq!(init_attr, Some(100.0));

        let billed_attr =
            find_attribute(span, "faas.billed_duration").and_then(|v| match &v.value {
                Some(Value::DoubleValue(d)) => Some(*d),
                _ => None,
            });
        assert_eq!(billed_attr, Some(200.0));

        let mem_attr = find_attribute(span, "faas.memory_used").and_then(|v| match &v.value {
            Some(Value::IntValue(i)) => Some(*i),
            _ => None,
        });
        assert_eq!(mem_attr, Some(128));

        // timestamps (ms -> ns)
        assert_eq!(span.start_time_unix_nano, 1_000_000_000);
        assert_eq!(span.end_time_unix_nano, 2_000_000_000);
    }
}

fn env_as_json_string() -> String {
    let map: JsonMap<String, serde_json::Value> = std::env::vars()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    serde_json::Value::Object(map).to_string()
}

fn get_trace_span_ids(invocation_id: &str, existing_traces: &[StoredTrace]) -> (Vec<u8>, Vec<u8>) {
    let mut found_trace_id: Option<Vec<u8>> = None;
    let mut found_parent_span_id: Option<Vec<u8>> = None;

    for trace in existing_traces {
        if !trace.invocation_ids.contains(&invocation_id.to_string()) {
            continue;
        }
        if let Ok(decoded) = ExportTraceServiceRequest::decode(trace.body.as_slice()) {
            tracing::info!(
                "[{}] found stored trace for invocation id {}",
                crate::log_prefix(),
                invocation_id,
            );
            if let Some(span) = decoded
                .resource_spans
                .into_iter()
                .flat_map(|rs| rs.scope_spans)
                .flat_map(|ss| ss.spans)
                .next()
            {
                if span.trace_id.len() == 16 {
                    found_trace_id = Some(span.trace_id.clone());
                }
                if !span.parent_span_id.is_empty() {
                    found_parent_span_id = Some(span.parent_span_id.clone());
                }
                crate::store::store_invocation_span_id(
                    invocation_id,
                    hex::encode(&span.trace_id),
                    hex::encode(&span.parent_span_id),
                );
                if found_trace_id.is_some() {
                    break;
                }
            }
        }
    }

    let trace_id = found_trace_id.unwrap_or_else(|| get_trace_id_from_invocation_id(invocation_id));

    let span_id =
        found_parent_span_id.unwrap_or_else(|| get_span_id_from_invocation_id(invocation_id));

    (trace_id, span_id)
}

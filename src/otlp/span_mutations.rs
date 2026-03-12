use crate::otlp::log_mutations::try_read_env_from_file;
use crate::state::invocation_data::StoredTrace;
use crate::state::invocation_entry;
use crate::util::parsers::{
    extract_invocation_id, get_span_id_from_invocation_id, get_span_scope_name,
    get_trace_id_from_invocation_id,
};
use hyper::header;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::span::{Event, Link, SpanKind};
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, Status};
use prost::Message;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn drop_duplicate_java_instrumenations(decoded: &ExportTraceServiceRequest) -> bool {
    let scope_name = get_span_scope_name(decoded);
    scope_name.as_deref() == Some("io.opentelemetry.aws-lambda-core-1.0")
}

pub fn build_synthetic_trace(
    invocation_id: &str,
    error_type: Option<&str>,
    return_value: Option<&str>,
    existing_traces: &[StoredTrace],
) -> Option<StoredTrace> {
    let (trace_id, span_id) = get_trace_span_ids(invocation_id, existing_traces);
    let parent_span_id = invocation_entry::get_root_span_id(invocation_id)
        .and_then(|id| hex::decode(&id).ok())
        .unwrap_or_default();

    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let start_nanos = invocation_entry::get_start_time(invocation_id)
        .map(|t| (t * 1_000_000.0) as u64)
        .unwrap_or(now_nanos);

    let mut attributes = crate::otlp::span_creation::get_span_attributes(invocation_id);

    let mut sqs_links = Vec::new();
    if let Some(event_payload) = invocation_entry::get_event_payload(invocation_id) {
        // Extract span links before consuming event_payload
        sqs_links = extract_span_links(&event_payload);

        attributes.extend(extract_span_attributes_from_event(&event_payload));
    } else {
        tracing::warn!(
            "[{}] No stored event payload found for invocation id {}",
            crate::log_prefix(),
            invocation_id
        );
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
        parent_span_id,
        name: "handler".to_string(),
        kind: SpanKind::Server as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: now_nanos,
        attributes,
        events,
        links: sqs_links,
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
        attributes: vec![KeyValue {
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
        }],
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

fn extract_span_links(event_payload: &str) -> Vec<Link> {
    crate::otlp::span_link_extractor::extract_span_links(event_payload)
}

fn add_resource_attributes(span: &mut Span) {
    if let Some(account_id) = crate::state::global::get_account_id() {
        span.attributes.push(KeyValue {
            key: "cloud.account.id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(account_id)),
            }),
        });
    }
    if let Some(function_arn) = crate::state::global::get_function_arn() {
        span.attributes.push(KeyValue {
            key: "cloud.resource_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(function_arn)),
            }),
        });
    }
}

fn extract_http_body_logs(span: &mut Span) {
    let invocation_id = match crate::state::invocation_data::get_current_invocation_id() {
        Some(id) => id,
        None => return,
    };

    const ONE_MS_NANOS: u64 = 1_000_000;
    let trace_id_hex = hex::encode(&span.trace_id);
    let span_id_hex = hex::encode(&span.span_id);

    let body_attr_keys: &[(&str, &str, u64)] = &[
        (
            "http.request.body",
            "http_request_body",
            span.start_time_unix_nano.saturating_add(ONE_MS_NANOS),
        ),
        (
            "http.response.body",
            "http_response_body",
            span.end_time_unix_nano.saturating_sub(ONE_MS_NANOS),
        ),
    ];

    for &(attr_key, payload_type, timestamp_nanos) in body_attr_keys {
        if let Some(value) = span.attributes.iter().find_map(|attr| {
            if attr.key == attr_key {
                if let Some(AnyValue {
                    value: Some(Value::StringValue(val)),
                }) = &attr.value
                {
                    return Some(val.clone());
                }
            }
            None
        }) {
            if let Some(log) = crate::otlp::log_mutations::build_payload_log(
                &value,
                payload_type,
                &invocation_id,
                Some(timestamp_nanos),
                Some(trace_id_hex.clone()),
                Some(span_id_hex.clone()),
            ) {
                invocation_entry::update(&invocation_id, |entry| {
                    entry.logs.push(log);
                });
            }
        }
    }

    span.attributes
        .retain(|attr| attr.key != "http.request.body" && attr.key != "http.response.body");
}

fn extract_span_attributes_from_event(event_payload: &str) -> Vec<KeyValue> {
    let mut attributes = Vec::new();

    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(event_payload) {
        if let Some(records) = json_val.get("Records").and_then(|v| v.as_array()) {
            attributes.push(KeyValue {
                key: "dash0.faas.record_count".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::IntValue(records.len() as i64)),
                }),
            });

            if let Some(first) = records.first() {
                if let Some(trigger) = first
                    .get("eventSource")
                    .or_else(|| first.get("EventSource"))
                    .and_then(|v| v.as_str())
                {
                    attributes.push(KeyValue {
                        key: "faas.trigger".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(trigger.to_string())),
                        }),
                    });
                }

                if let Some(arn) = first
                    .get("eventSourceARN")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        first
                            .get("Sns")
                            .and_then(|sns| sns.get("TopicArn"))
                            .and_then(|v| v.as_str())
                    })
                {
                    attributes.push(KeyValue {
                        key: "dash0.faas.trigger_arn".to_string(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(arn.to_string())),
                        }),
                    });
                }
            }
        } else if json_val.get("source").and_then(|v| v.as_str()).is_some()
            && json_val
                .get("detail-type")
                .and_then(|v| v.as_str())
                .is_some()
        {
            attributes.push(KeyValue {
                key: "faas.trigger".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue("aws:event_bridge".to_string())),
                }),
            });
            if let Some(source) = json_val.get("source").and_then(|v| v.as_str()) {
                attributes.push(KeyValue {
                    key: "dash0.faas.event_bridge_source".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue(source.to_string())),
                    }),
                });
            }
            if let Some(detail_type) = json_val.get("detail-type").and_then(|v| v.as_str()) {
                attributes.push(KeyValue {
                    key: "dash0.faas.event_bridge_detail_type".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue(detail_type.to_string())),
                    }),
                });
            }
        }
    }

    attributes
}

fn add_event_payload_to_span(span: &mut Span, invocation_id: &str) {
    if let Some(event_payload) = invocation_entry::get_event_payload(invocation_id) {
        let sqs_links = extract_span_links(&event_payload);
        if !sqs_links.is_empty() {
            tracing::trace!(
                "[{}] Adding {} SQS span links to lambda span for invocation_id={}",
                crate::log_prefix(),
                sqs_links.len(),
                invocation_id
            );
            span.links.extend(sqs_links);
        }

        span.attributes
            .extend(extract_span_attributes_from_event(&event_payload));
    } else {
        tracing::warn!(
            "[{}] No stored event payload found for invocation id {}",
            crate::log_prefix(),
            invocation_id
        );
    }
}

fn reparent_to_root_span(span: &mut Span, invocation_id: &str) {
    if let Some(root_span_id) = invocation_entry::get_root_span_id(invocation_id) {
        if let Ok(bytes) = hex::decode(&root_span_id) {
            span.parent_span_id = bytes;
        }
    }
}

fn store_span_ids(span: &Span, invocation_id: &str) {
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
    invocation_entry::update(invocation_id, |entry| {
        entry.trace_id = Some(trace_id_hex);
        entry.span_id = Some(span_id_hex);
    });
    tracing::debug!(
        "[{}] stored trace/span id for invocation_id={}",
        crate::log_prefix(),
        invocation_id
    );
}

/// Process a decoded trace request by adding event payloads, return payloads, and storing invocation span IDs.
pub fn process_trace_request(
    decoded: &mut ExportTraceServiceRequest,
    invocation_ids: &mut Vec<String>,
    encoded_body: &mut Vec<u8>,
) {
    for resource_span in &mut decoded.resource_spans {
        for scope_span in &mut resource_span.scope_spans {
            let is_lambda = scope_span
                .scope
                .as_ref()
                .map_or(false, |s| is_lambda_instrumentation_scope(&s.name));
            if !is_lambda {
                for span in &mut scope_span.spans {
                    add_resource_attributes(span);
                    extract_http_body_logs(span);
                }
                continue;
            }

            for span in &mut scope_span.spans {
                let invocation_id = match extract_invocation_id(span) {
                    Some(id) => id,
                    None => continue,
                };

                invocation_ids.push(invocation_id.clone());
                add_event_payload_to_span(span, &invocation_id);
                reparent_to_root_span(span, &invocation_id);
                store_span_ids(span, &invocation_id);
            }
        }
    }
    *encoded_body = decoded.encode_to_vec();
}

#[cfg(test)]
mod tests {
    use super::{build_synthetic_trace, StatusCode};
    use crate::state::invocation_data::StoredTrace;
    use crate::state::invocation_entry;
    use hyper::{header, Method};
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use prost::Message;
    use serial_test::serial;

    fn find_attribute<'a>(span: &'a Span, key: &'a str) -> Option<&'a AnyValue> {
        span.attributes
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
    }

    fn store_event_payload(invocation_id: &str, payload: &str) {
        let payload = payload.to_string();
        invocation_entry::update(invocation_id, |entry| {
            entry.event_payload = Some(payload);
        });
    }

    fn store_trace(invocation_id: &str, trace: StoredTrace) {
        invocation_entry::store_trace_by_id(invocation_id, trace);
    }

    fn take_traces() -> Vec<StoredTrace> {
        invocation_entry::take_all_traces()
    }

    fn snapshot_traces() -> Vec<StoredTrace> {
        invocation_entry::snapshot_all_traces()
    }

    #[test]
    #[serial]
    fn builds_trace_with_event_payload_and_invocation_id() {
        let invocation_id = "inv-test-1";
        store_event_payload(invocation_id, r#"{"foo":"bar"}"#);

        let trace = build_synthetic_trace(invocation_id, Some("CustomError"), None, &[])
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

        let trace = build_synthetic_trace(invocation_id, Some("error"), None, &[])
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
    fn build_synthetic_trace_uses_existing_trace_and_parent_ids() {
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
        store_trace(invocation_id, trace);

        let traces = snapshot_traces();
        let synthetic = build_synthetic_trace(invocation_id, Some("CopiedError"), None, &traces)
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

        super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        assert!(!encoded_body.is_empty(), "encoded_body should be updated");
    }

    #[test]
    #[serial]
    fn process_trace_request_keeps_parent_span_when_env_var_false() {
        std::env::set_var("DASH0_REMOVE_LAMBDA_PARENT_SPAN", "false");
        let invocation_id = "inv-process-parent-env";
        store_event_payload(invocation_id, r#"{"test":"data"}"#);

        let mut span = make_span_with_invocation(invocation_id);
        span.parent_span_id = vec![0xBB; 8];

        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        let span = &request.resource_spans[0].scope_spans[0].spans[0];
        assert_eq!(
            span.parent_span_id,
            vec![0xBB; 8],
            "parent_span_id should be preserved when DASH0_REMOVE_LAMBDA_PARENT_SPAN is false"
        );
        std::env::remove_var("DASH0_REMOVE_LAMBDA_PARENT_SPAN");
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
        let stored = invocation_entry::get(invocation_id);
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

        assert_eq!(stored.trace_id.unwrap(), expected_trace_id);
        assert_eq!(stored.span_id.unwrap(), expected_span_id);
    }

    #[test]
    #[serial]
    fn process_trace_request_no_modifications_still_encodes() {
        let invocation_id = "inv-process-5";

        // Don't store any event payload or return payload
        let span = make_span_with_invocation(invocation_id);
        let mut request = make_request_with_scope("opentelemetry.instrumentation.aws_lambda", span);
        let mut invocation_ids = Vec::new();
        let mut encoded_body = Vec::new();

        super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

        assert_eq!(invocation_ids, vec![invocation_id.to_string()]);
        assert!(
            !encoded_body.is_empty(),
            "encoded_body should always be set"
        );
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

        super::process_trace_request(&mut request, &mut invocation_ids, &mut encoded_body);

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
    }

    #[test]
    #[serial]
    fn builds_trace_with_error_from_return_value() {
        let invocation_id = "inv-error-json";
        let error_json = r#"{"errorMessage": "Something went wrong", "errorType": "ValueError", "requestId": "123", "stackTrace": ["line1\n", "line2\n"]}"#;

        let trace = build_synthetic_trace(invocation_id, None, Some(error_json), &[])
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

    // --- get_trace_span_ids tests ---

    fn build_stored_trace_with_ids(
        invocation_id: &str,
        trace_id: Vec<u8>,
        span_id: Vec<u8>,
        parent_span_id: Vec<u8>,
    ) -> StoredTrace {
        let span = Span {
            trace_id,
            span_id,
            parent_span_id,
            ..Default::default()
        };
        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: hyper::HeaderMap::new(),
            body: request.encode_to_vec(),
            invocation_ids: vec![invocation_id.to_string()],
        }
    }

    #[test]
    #[serial]
    fn get_trace_span_ids_from_existing_trace() {
        let invocation_id = "inv-ids-from-trace";
        let trace_id = vec![0xAA; 16];
        let span_id = vec![0xBB; 8];
        let parent_span_id = vec![0xCC; 8];

        let stored_trace = build_stored_trace_with_ids(
            invocation_id,
            trace_id.clone(),
            span_id.clone(),
            parent_span_id.clone(),
        );

        let (got_trace, got_span) = super::get_trace_span_ids(invocation_id, &[stored_trace]);

        assert_eq!(got_trace, trace_id);
        // span_id is taken from the existing span's parent_span_id
        assert_eq!(got_span, parent_span_id);
    }

    #[test]
    #[serial]
    fn get_trace_span_ids_from_stored_entry() {
        let invocation_id = "inv-ids-from-entry";
        let trace_id_hex = "aa".repeat(16);
        let span_id_hex = "bb".repeat(8);

        invocation_entry::update(invocation_id, |entry| {
            entry.trace_id = Some(trace_id_hex.clone());
            entry.span_id = Some(span_id_hex.clone());
        });

        let (got_trace, got_span) = super::get_trace_span_ids(invocation_id, &[]);

        assert_eq!(hex::encode(&got_trace), trace_id_hex);
        assert_eq!(hex::encode(&got_span), span_id_hex);
    }

    #[test]
    #[serial]
    fn get_trace_span_ids_falls_back_to_hash() {
        let invocation_id = "inv-ids-hash-fallback";
        // No traces, no stored entry → should generate from hash
        let (got_trace, got_span) = super::get_trace_span_ids(invocation_id, &[]);

        let expected_trace = crate::util::parsers::get_trace_id_from_invocation_id(invocation_id);
        let expected_span = crate::util::parsers::get_span_id_from_invocation_id(invocation_id);

        assert_eq!(got_trace, expected_trace);
        assert_eq!(got_span, expected_span);
    }

    #[test]
    #[serial]
    fn get_trace_span_ids_trace_overrides_stored_entry() {
        let invocation_id = "inv-ids-trace-wins";
        let trace_trace_id = vec![0x11; 16];
        let trace_parent_span_id = vec![0x22; 8];

        // Store different IDs in the invocation entry
        invocation_entry::update(invocation_id, |entry| {
            entry.trace_id = Some("ff".repeat(16));
            entry.span_id = Some("ee".repeat(8));
            entry.parent_span_id = Some("dd".repeat(8));
        });

        let stored_trace = build_stored_trace_with_ids(
            invocation_id,
            trace_trace_id.clone(),
            vec![0x33; 8], // span's own span_id (not used directly)
            trace_parent_span_id.clone(),
        );

        let (got_trace, got_span) = super::get_trace_span_ids(invocation_id, &[stored_trace]);

        // trace_id and span_id should come from the existing trace, not the stored entry
        assert_eq!(got_trace, trace_trace_id);
        assert_eq!(got_span, trace_parent_span_id);
    }

    #[test]
    #[serial]
    fn get_trace_span_ids_skips_non_matching_traces() {
        let invocation_id = "inv-ids-no-match";
        let other_trace = build_stored_trace_with_ids(
            "other-invocation",
            vec![0xAA; 16],
            vec![0xBB; 8],
            vec![0xCC; 8],
        );

        let (got_trace, got_span) = super::get_trace_span_ids(invocation_id, &[other_trace]);

        // Should fall through to hash-based generation
        let expected_trace = crate::util::parsers::get_trace_id_from_invocation_id(invocation_id);
        let expected_span = crate::util::parsers::get_span_id_from_invocation_id(invocation_id);

        assert_eq!(got_trace, expected_trace);
        assert_eq!(got_span, expected_span);
    }

    #[test]
    #[serial]
    fn get_trace_span_ids_updates_invocation_entry() {
        let invocation_id = "inv-ids-updates-entry";
        let trace_id = vec![0xDE; 16];
        let parent_span_id = vec![0xAD; 8];

        let stored_trace = build_stored_trace_with_ids(
            invocation_id,
            trace_id.clone(),
            vec![0xFF; 8],
            parent_span_id.clone(),
        );

        super::get_trace_span_ids(invocation_id, &[stored_trace]);

        let entry = invocation_entry::get(invocation_id).expect("entry should exist after call");
        assert_eq!(entry.trace_id.unwrap(), hex::encode(&trace_id));
        assert_eq!(entry.span_id.unwrap(), hex::encode(&parent_span_id));
    }

    // --- extract_span_attributes_from_event tests ---

    fn find_extracted_attr<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a AnyValue> {
        attrs
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
    }

    fn get_string_attr(attrs: &[KeyValue], key: &str) -> Option<String> {
        find_extracted_attr(attrs, key).and_then(|v| match &v.value {
            Some(Value::StringValue(s)) => Some(s.clone()),
            _ => None,
        })
    }

    fn get_int_attr(attrs: &[KeyValue], key: &str) -> Option<i64> {
        find_extracted_attr(attrs, key).and_then(|v| match &v.value {
            Some(Value::IntValue(i)) => Some(*i),
            _ => None,
        })
    }

    #[test]
    fn extract_span_attributes_json_without_records() {
        let attrs = super::extract_span_attributes_from_event(r#"{"key":"value"}"#);
        assert!(attrs.is_empty());
        assert!(find_extracted_attr(&attrs, "dash0.faas.record_count").is_none());
    }

    #[test]
    fn extract_span_attributes_empty_records() {
        let attrs = super::extract_span_attributes_from_event(r#"{"Records":[]}"#);
        assert_eq!(get_int_attr(&attrs, "dash0.faas.record_count"), Some(0));
        assert!(find_extracted_attr(&attrs, "faas.trigger").is_none());
        assert!(find_extracted_attr(&attrs, "dash0.faas.trigger_arn").is_none());
    }

    #[test]
    fn extract_span_attributes_sqs_event() {
        let payload = r#"{"Records":[{"eventSource":"aws:sqs","eventSourceARN":"arn:aws:sqs:us-east-1:123:my-queue","body":"hello"},{"eventSource":"aws:sqs","body":"world"}]}"#;
        let attrs = super::extract_span_attributes_from_event(payload);
        assert_eq!(get_int_attr(&attrs, "dash0.faas.record_count"), Some(2));
        assert_eq!(
            get_string_attr(&attrs, "faas.trigger"),
            Some("aws:sqs".to_string())
        );
        assert_eq!(
            get_string_attr(&attrs, "dash0.faas.trigger_arn"),
            Some("arn:aws:sqs:us-east-1:123:my-queue".to_string())
        );
    }

    #[test]
    fn extract_span_attributes_sns_event() {
        let payload = r#"{"Records":[{"EventSource":"aws:sns","Sns":{"TopicArn":"arn:aws:sns:us-east-1:123:my-topic","Message":"hello"}}]}"#;
        let attrs = super::extract_span_attributes_from_event(payload);
        assert_eq!(get_int_attr(&attrs, "dash0.faas.record_count"), Some(1));
        assert_eq!(
            get_string_attr(&attrs, "faas.trigger"),
            Some("aws:sns".to_string())
        );
        assert_eq!(
            get_string_attr(&attrs, "dash0.faas.trigger_arn"),
            Some("arn:aws:sns:us-east-1:123:my-topic".to_string())
        );
    }

    #[test]
    fn extract_span_attributes_record_without_source_or_arn() {
        let payload = r#"{"Records":[{"body":"data"}]}"#;
        let attrs = super::extract_span_attributes_from_event(payload);
        assert_eq!(get_int_attr(&attrs, "dash0.faas.record_count"), Some(1));
        assert!(find_extracted_attr(&attrs, "faas.trigger").is_none());
        assert!(find_extracted_attr(&attrs, "dash0.faas.trigger_arn").is_none());
    }

    #[test]
    fn extract_span_attributes_eventbridge_event() {
        let payload = r#"{"version":"0","source":"aws.ec2","detail-type":"EC2 Instance State-change Notification","account":"123456789012","region":"us-east-1","detail":{}}"#;
        let attrs = super::extract_span_attributes_from_event(payload);
        assert_eq!(
            get_string_attr(&attrs, "faas.trigger"),
            Some("aws:event_bridge".to_string())
        );
        assert_eq!(
            get_string_attr(&attrs, "dash0.faas.event_bridge_source"),
            Some("aws.ec2".to_string())
        );
        assert_eq!(
            get_string_attr(&attrs, "dash0.faas.event_bridge_detail_type"),
            Some("EC2 Instance State-change Notification".to_string())
        );
        assert!(find_extracted_attr(&attrs, "dash0.faas.record_count").is_none());
    }

    #[test]
    fn extract_span_attributes_eventbridge_not_detected_with_records() {
        // If Records is present, it should be treated as a Records-based event, not EventBridge
        let payload = r#"{"Records":[{"eventSource":"aws:sqs"}],"source":"aws.ec2","detail-type":"something"}"#;
        let attrs = super::extract_span_attributes_from_event(payload);
        assert_eq!(
            get_string_attr(&attrs, "faas.trigger"),
            Some("aws:sqs".to_string())
        );
        assert!(find_extracted_attr(&attrs, "dash0.faas.event_bridge_source").is_none());
    }

    #[test]
    fn extract_span_attributes_eventbridge_requires_both_fields() {
        // Only "source" without "detail-type" should not match EventBridge
        let payload = r#"{"source":"aws.ec2","account":"123"}"#;
        let attrs = super::extract_span_attributes_from_event(payload);
        assert!(find_extracted_attr(&attrs, "faas.trigger").is_none());
        assert!(find_extracted_attr(&attrs, "dash0.faas.event_bridge_source").is_none());
    }

    // --- extract_http_body_logs tests ---

    use crate::state::invocation_data::TelemetryLog;

    fn enable_payload_log_records() {
        std::env::set_var("DASH0_CREATE_PAYLOAD_LOG_RECORDS", "true");
    }

    fn disable_payload_log_records() {
        std::env::set_var("DASH0_CREATE_PAYLOAD_LOG_RECORDS", "false");
    }

    fn set_current_invocation(id: &str) {
        crate::state::invocation_data::store_current_invocation_id(id);
    }

    fn take_logs_for(invocation_id: &str) -> Vec<TelemetryLog> {
        invocation_entry::remove(invocation_id)
            .map(|e| e.logs)
            .unwrap_or_default()
    }

    fn make_http_span(
        request_body: Option<&str>,
        response_body: Option<&str>,
        start_nanos: u64,
        end_nanos: u64,
    ) -> Span {
        let mut attributes = Vec::new();
        if let Some(body) = request_body {
            attributes.push(KeyValue {
                key: "http.request.body".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(body.to_string())),
                }),
            });
        }
        if let Some(body) = response_body {
            attributes.push(KeyValue {
                key: "http.response.body".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(body.to_string())),
                }),
            });
        }
        Span {
            trace_id: hex::decode("5b8eff129842a1b9c9283745a23f54b1").unwrap(),
            span_id: hex::decode("023f54b19283745a").unwrap(),
            start_time_unix_nano: start_nanos,
            end_time_unix_nano: end_nanos,
            attributes,
            ..Default::default()
        }
    }

    fn parse_payload_log(log: &TelemetryLog) -> serde_json::Value {
        match &log.record {
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap(),
            other => other.clone(),
        }
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_creates_request_and_response_logs() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-1";
        set_current_invocation(inv_id);

        let start = 1_700_000_000_000_000_000u64; // some timestamp in nanos
        let end = 1_700_000_001_000_000_000u64; // 1 second later
        let mut span = make_http_span(
            Some(r#"{"key":"value"}"#),
            Some(r#"{"status":"ok"}"#),
            start,
            end,
        );

        super::extract_http_body_logs(&mut span);

        // Both body attributes should be removed from span
        assert!(
            span.attributes.is_empty(),
            "http body attributes should be removed from span"
        );

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 2, "should create 2 logs (request + response)");

        // Request log
        let req_log = &logs[0];
        let req_parsed = parse_payload_log(req_log);
        assert_eq!(req_parsed["name"], "dash0_payload");
        assert_eq!(req_parsed["type"], "http_request_body");
        assert_eq!(req_parsed["message"], serde_json::json!({"key":"value"}));
        assert_eq!(
            req_log.trace_id.as_deref(),
            Some("5b8eff129842a1b9c9283745a23f54b1")
        );
        assert_eq!(req_log.span_id.as_deref(), Some("023f54b19283745a"));

        // Response log
        let resp_log = &logs[1];
        let resp_parsed = parse_payload_log(resp_log);
        assert_eq!(resp_parsed["name"], "dash0_payload");
        assert_eq!(resp_parsed["type"], "http_response_body");
        assert_eq!(resp_parsed["message"], serde_json::json!({"status":"ok"}));
        assert_eq!(
            resp_log.trace_id.as_deref(),
            Some("5b8eff129842a1b9c9283745a23f54b1")
        );
        assert_eq!(resp_log.span_id.as_deref(), Some("023f54b19283745a"));

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_uses_correct_timestamps() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-ts";
        set_current_invocation(inv_id);

        let start = 1_700_000_000_000_000_000u64;
        let end = 1_700_000_001_000_000_000u64;
        let mut span = make_http_span(Some("req"), Some("resp"), start, end);

        super::extract_http_body_logs(&mut span);

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 2);

        // Request timestamp should be start + 1ms
        let req_time = chrono::DateTime::parse_from_rfc3339(&logs[0].time).unwrap();
        let expected_req_nanos = start + 1_000_000;
        assert_eq!(
            req_time.timestamp_nanos_opt().unwrap() as u64,
            expected_req_nanos,
            "request log timestamp should be span start + 1ms"
        );

        // Response timestamp should be end - 1ms
        let resp_time = chrono::DateTime::parse_from_rfc3339(&logs[1].time).unwrap();
        let expected_resp_nanos = end - 1_000_000;
        assert_eq!(
            resp_time.timestamp_nanos_opt().unwrap() as u64,
            expected_resp_nanos,
            "response log timestamp should be span end - 1ms"
        );

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_only_request_body() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-req-only";
        set_current_invocation(inv_id);

        let mut span = make_http_span(Some("request data"), None, 100, 200);

        super::extract_http_body_logs(&mut span);

        assert!(span.attributes.is_empty());

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 1);
        let parsed = parse_payload_log(&logs[0]);
        assert_eq!(parsed["type"], "http_request_body");

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_only_response_body() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-resp-only";
        set_current_invocation(inv_id);

        let mut span = make_http_span(None, Some("response data"), 100, 200);

        super::extract_http_body_logs(&mut span);

        assert!(span.attributes.is_empty());

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 1);
        let parsed = parse_payload_log(&logs[0]);
        assert_eq!(parsed["type"], "http_response_body");

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_no_body_attributes() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-none";
        set_current_invocation(inv_id);

        let mut span = make_http_span(None, None, 100, 200);
        // Add some other attribute to verify it's preserved
        span.attributes.push(KeyValue {
            key: "http.method".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("GET".to_string())),
            }),
        });

        super::extract_http_body_logs(&mut span);

        assert_eq!(
            span.attributes.len(),
            1,
            "non-body attributes should be preserved"
        );
        assert_eq!(span.attributes[0].key, "http.method");

        let logs = take_logs_for(inv_id);
        assert!(
            logs.is_empty(),
            "no logs should be created when no body attributes"
        );

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_preserves_other_attributes() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-preserve";
        set_current_invocation(inv_id);

        let mut span = make_http_span(Some("req"), Some("resp"), 100, 200);
        span.attributes.push(KeyValue {
            key: "http.status_code".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(200)),
            }),
        });
        span.attributes.push(KeyValue {
            key: "http.url".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("https://example.com".to_string())),
            }),
        });

        super::extract_http_body_logs(&mut span);

        assert_eq!(
            span.attributes.len(),
            2,
            "only body attributes should be removed"
        );
        let keys: Vec<&str> = span.attributes.iter().map(|a| a.key.as_str()).collect();
        assert!(keys.contains(&"http.status_code"));
        assert!(keys.contains(&"http.url"));

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 2);

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_disabled_creates_no_logs() {
        disable_payload_log_records();
        let inv_id = "inv-http-body-disabled";
        set_current_invocation(inv_id);

        let mut span = make_http_span(Some("req"), Some("resp"), 100, 200);

        super::extract_http_body_logs(&mut span);

        // Attributes should still be removed even when log creation is disabled
        assert!(span.attributes.is_empty());

        let logs = take_logs_for(inv_id);
        assert!(
            logs.is_empty(),
            "no logs when DASH0_CREATE_PAYLOAD_LOG_RECORDS is not set"
        );
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_handles_non_json_body() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-nonjson";
        set_current_invocation(inv_id);

        let mut span = make_http_span(Some("plain text body"), None, 100, 200);

        super::extract_http_body_logs(&mut span);

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 1);
        let parsed = parse_payload_log(&logs[0]);
        // Non-JSON should be wrapped as a JSON string
        assert_eq!(parsed["message"], "plain text body");

        disable_payload_log_records();
    }

    #[test]
    #[serial]
    fn extract_http_body_logs_invocation_id_set_on_log() {
        enable_payload_log_records();
        let inv_id = "inv-http-body-invid";
        set_current_invocation(inv_id);

        let mut span = make_http_span(Some("req"), None, 100, 200);

        super::extract_http_body_logs(&mut span);

        let logs = take_logs_for(inv_id);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].invocation_id.as_deref(), Some(inv_id));

        disable_payload_log_records();
    }
}

fn get_trace_span_ids(invocation_id: &str, existing_traces: &[StoredTrace]) -> (Vec<u8>, Vec<u8>) {
    let mut trace_id: Option<Vec<u8>> = None;
    let mut span_id: Option<Vec<u8>> = None;

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
                    trace_id = Some(span.trace_id);
                }
                if !span.parent_span_id.is_empty() {
                    span_id = Some(span.parent_span_id);
                }
            }
        }
    }

    let stored_ids = invocation_entry::get_trace_span_ids(invocation_id);

    let trace_id = trace_id.unwrap_or_else(|| {
        stored_ids
            .as_ref()
            .and_then(|(t, _, _)| t.as_deref())
            .filter(|t| !t.is_empty())
            .and_then(|t| hex::decode(t).ok())
            .filter(|t| t.len() == 16)
            .unwrap_or_else(|| get_trace_id_from_invocation_id(invocation_id))
    });

    let span_id = span_id.unwrap_or_else(|| {
        stored_ids
            .as_ref()
            .and_then(|(_, s, _)| s.as_deref())
            .filter(|s| !s.is_empty())
            .and_then(|s| hex::decode(s).ok())
            .filter(|p| p.len() == 8)
            .unwrap_or_else(|| get_span_id_from_invocation_id(invocation_id))
    });

    invocation_entry::update(invocation_id, |entry| {
        entry.trace_id = Some(hex::encode(&trace_id));
        entry.span_id = Some(hex::encode(&span_id));
    });

    (trace_id, span_id)
}

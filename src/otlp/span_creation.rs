use crate::otlp::log_mutations::try_read_env_from_file;
use crate::state::invocation_data::StoredTrace;
use crate::state::invocation_entry;
use hyper::header;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message;

pub fn get_span_attributes(invocation_id: &str) -> Vec<KeyValue> {
    vec![
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
                    crate::state::global::get_function_arn()
                        .unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
        KeyValue {
            key: "cloud.account.id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_account_id()
                        .unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
    ]
}

fn create_root_span(
    invocation_id: &str,
    data: &invocation_entry::SupplementarySpanData,
) -> Option<Span> {
    let root_span_id = data.root_span_id.as_deref()?;
    let trace_id_hex = data.trace_id.as_deref().filter(|s| !s.is_empty())?;

    let span_id = hex::decode(root_span_id).ok()?;
    let trace_id = hex::decode(trace_id_hex).ok()?;

    let parent_span_id =
        if data.sampled || !crate::config::user::is_remove_lambda_parent_span() {
            data.parent_span_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|p| hex::decode(p).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

    let start_nanos = (data.start_time * 1_000_000.0) as u64;
    let end_nanos = start_nanos + (data.billed_duration * 1_000_000.0) as u64;

    let function_name =
        std::env::var("AWS_LAMBDA_FUNCTION_NAME").unwrap_or_else(|_| "unknown".to_string());

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        name: function_name,
        kind: SpanKind::Server as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: end_nanos,
        attributes: get_span_attributes(invocation_id),
        ..Default::default()
    })
}

pub fn create_supplementary_spans(invocation_id: &str) {
    let data = match invocation_entry::get_supplementary_span_data(invocation_id) {
        Some(d) => d,
        None => return,
    };

    let mut spans = Vec::new();

    if let Some(root_span) = create_root_span(invocation_id, &data) {
        spans.push(root_span);
    }

    if spans.is_empty() {
        return;
    }

    let scope_spans = ScopeSpans {
        scope: Some(InstrumentationScope {
            name: "opentelemetry.instrumentation.aws_lambda".to_string(),
            version: "unknown".to_string(),
            ..Default::default()
        }),
        spans,
        schema_url: "https://opentelemetry.io/schemas/1.11.0".to_string(),
    };

    let resource = Resource {
        attributes: vec![KeyValue {
            key: "service.name".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    std::env::var("OTEL_SERVICE_NAME")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .or_else(|| try_read_env_from_file("OTEL_SERVICE_NAME"))
                        .unwrap_or_else(|| "unknown_service".to_string()),
                )),
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

    let trace = StoredTrace {
        method: hyper::Method::POST,
        path_and_query: "/v1/traces".to_string(),
        headers,
        body: export.encode_to_vec(),
        invocation_ids: vec![invocation_id.to_string()],
    };

    invocation_entry::store_trace_by_id(invocation_id, trace);
}

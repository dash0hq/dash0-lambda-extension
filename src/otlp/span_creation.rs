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

pub fn create_supplementary_spans(invocation_id: &str) {
    let data = match invocation_entry::get_supplementary_span_data(invocation_id) {
        Some(d) => d,
        None => return,
    };

    let root_span_id = match data.root_span_id.as_deref() {
        Some(id) => id,
        None => return,
    };
    let trace_id_hex = match data.trace_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => return,
    };

    let span_id = match hex::decode(root_span_id) {
        Ok(b) => b,
        Err(_) => return,
    };
    let trace_id = match hex::decode(trace_id_hex) {
        Ok(b) => b,
        Err(_) => return,
    };

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

    let function_name = std::env::var("AWS_LAMBDA_FUNCTION_NAME")
        .unwrap_or_else(|_| "unknown".to_string());

    let span = Span {
        trace_id,
        span_id,
        parent_span_id,
        name: function_name,
        kind: SpanKind::Server as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: end_nanos,
        attributes: vec![KeyValue {
            key: "faas.invocation_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(invocation_id.to_string())),
            }),
        }],
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

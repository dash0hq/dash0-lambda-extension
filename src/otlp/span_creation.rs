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
    use crate::otlp::attributes::*;
    vec![
        KeyValue {
            key: FAAS_INVOCATION_ID.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(invocation_id.to_string())),
            }),
        },
        KeyValue {
            key: CLOUD_RESOURCE_ID.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_function_arn()
                        .unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
        KeyValue {
            key: CLOUD_ACCOUNT_ID.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_account_id().unwrap_or_else(|| "unknown".to_string()),
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

    let parent_span_id = data
        .parent_span_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|p| hex::decode(p).ok())
        .unwrap_or_default();

    let start_nanos = ((data.start_time - data.init_duration) * 1_000_000.0) as u64;
    let end_nanos = (data.end_time * 1_000_000.0) as u64;

    let function_name =
        std::env::var("AWS_LAMBDA_FUNCTION_NAME").unwrap_or_else(|_| "unknown".to_string());

    let mut attrs = get_span_attributes(invocation_id);
    if data.init_duration > 0.0 {
        attrs.push(KeyValue {
            key: crate::otlp::attributes::FAAS_INIT_DURATION.to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(data.init_duration)),
            }),
        });
    }
    attrs.extend(data.handler_attributes.clone());

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        name: function_name,
        kind: SpanKind::Server as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: end_nanos,
        attributes: attrs,
        status: data.handler_status.clone(),
        ..Default::default()
    })
}

fn create_init_span(
    invocation_id: &str,
    data: &invocation_entry::SupplementarySpanData,
) -> Option<Span> {
    if data.init_duration <= 0.0 {
        return None;
    }

    let root_span_id = data.root_span_id.as_deref()?;
    let trace_id_hex = data.trace_id.as_deref().filter(|s| !s.is_empty())?;

    let parent_span_id = hex::decode(root_span_id).ok()?;
    let trace_id = hex::decode(trace_id_hex).ok()?;

    let span_id = crate::util::parsers::generate_random_span_id();

    let start_nanos = ((data.start_time - data.init_duration) * 1_000_000.0) as u64;
    let end_nanos = ((data.start_time - 1.0) * 1_000_000.0) as u64;

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        name: "aws.lambda.initialization".to_string(),
        kind: SpanKind::Internal as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: end_nanos,
        attributes: get_span_attributes(invocation_id),
        ..Default::default()
    })
}

fn create_overhead_span(
    invocation_id: &str,
    data: &invocation_entry::SupplementarySpanData,
) -> Option<Span> {
    let root_span_id = data.root_span_id.as_deref()?;
    let trace_id_hex = data.trace_id.as_deref().filter(|s| !s.is_empty())?;

    let parent_span_id = hex::decode(root_span_id).ok()?;
    let trace_id = hex::decode(trace_id_hex).ok()?;
    let span_id = crate::util::parsers::generate_random_span_id();

    let start_nanos = (data.end_time * 1_000_000.0) as u64;
    let end_nanos =
        ((data.start_time - data.init_duration + data.billed_duration) * 1_000_000.0) as u64;

    let end_nanos = end_nanos.max(start_nanos);

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        name: "aws.lambda.overhead".to_string(),
        kind: SpanKind::Internal as i32,
        start_time_unix_nano: start_nanos,
        end_time_unix_nano: end_nanos,
        attributes: get_span_attributes(invocation_id),
        ..Default::default()
    })
}

pub fn create_spans(
    invocation_id: &str,
    create_root_and_init: bool,
    create_overhead: bool,
) -> Option<StoredTrace> {
    let data = invocation_entry::get_supplementary_span_data(invocation_id)?;

    let mut spans = Vec::new();

    if create_root_and_init {
        if let Some(root_span) = create_root_span(invocation_id, &data) {
            spans.push(root_span);
        }

        if let Some(init_span) = create_init_span(invocation_id, &data) {
            spans.push(init_span);
        }
    }

    if create_overhead {
        if let Some(overhead_span) = create_overhead_span(invocation_id, &data) {
            spans.push(overhead_span);
        }
    }

    if spans.is_empty() {
        tracing::info!(
            "[{}] No spans created for invocation {}, skipping trace creation. data: {:?}",
            crate::log_prefix(),
            invocation_id,
            data
        );
        return None;
    }

    let scope_spans = ScopeSpans {
        scope: Some(InstrumentationScope {
            name: "dash0.lambda-extension".to_string(),
            version: "1.0".to_string(),
            ..Default::default()
        }),
        spans,
        schema_url: crate::otlp::OTEL_SCHEMA_URL.to_string(),
    };

    let resource = Resource {
        attributes: crate::otlp::resources::get_resources_attributes(),
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

pub fn create_supplementary_spans(invocation_id: &str, store: bool) -> Option<StoredTrace> {
    if crate::config::user::is_xray_traces_enabled()
        || crate::config::user::is_telemetry_traces_disabled()
    {
        return None;
    }
    let trace = create_spans(invocation_id, true, false);
    if store {
        if let Some(ref t) = trace {
            invocation_entry::store_trace_by_id(invocation_id, t.clone());
        }
    }
    trace
}

pub fn create_overhead_supplementary_span(invocation_id: &str) {
    if crate::config::user::is_xray_traces_enabled()
        || crate::config::user::is_telemetry_traces_disabled()
    {
        return;
    }
    if let Some(trace) = create_spans(invocation_id, false, true) {
        invocation_entry::store_trace_by_id(invocation_id, trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::invocation_entry;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use prost::Message;
    use serial_test::serial;

    fn reset_store() {
        // Remove test entries
        invocation_entry::remove("inv-supp-1");
    }

    #[test]
    #[serial]
    fn create_supplementary_spans_stores_root_span() {
        reset_store();
        let invocation_id = "inv-supp-1";
        let trace_id_hex = "aa".repeat(16);
        let root_span_id_hex = "bb".repeat(8);
        let parent_span_id_hex = "cc".repeat(8);

        invocation_entry::update(invocation_id, |entry| {
            entry.trace_id = Some(trace_id_hex.clone());
            entry.root_span_id = Some(root_span_id_hex.clone());
            entry.parent_span_id = Some(parent_span_id_hex.clone());
            entry.sampled = true;
            entry.start_time = 1_000.0; // 1 second in ms
            entry.billed_duration = 500.0; // 500ms
            entry.init_duration = 150.0;
            entry.memory_usage = 256;
            entry.end_time = 1_100.0; // 1100ms
        });

        std::env::set_var("AWS_LAMBDA_FUNCTION_NAME", "my-function");
        create_supplementary_spans(invocation_id, true);
        std::env::remove_var("AWS_LAMBDA_FUNCTION_NAME");

        let traces = invocation_entry::take_traces_by_id(invocation_id);
        assert_eq!(traces.len(), 1);

        let trace = &traces[0];
        assert_eq!(trace.path_and_query, "/v1/traces");
        assert_eq!(trace.invocation_ids, vec![invocation_id.to_string()]);

        let decoded =
            ExportTraceServiceRequest::decode(trace.body.as_slice()).expect("should decode");
        let span = &decoded.resource_spans[0].scope_spans[0].spans[0];

        assert_eq!(hex::encode(&span.span_id), root_span_id_hex);
        assert_eq!(hex::encode(&span.trace_id), trace_id_hex);
        assert_eq!(hex::encode(&span.parent_span_id), parent_span_id_hex);
        assert_eq!(span.name, "my-function");
        assert_eq!(span.start_time_unix_nano, 850_000_000); // (1000 - 150) ms in nanos
        assert_eq!(span.end_time_unix_nano, 1_100_000_000);

        let inv_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key == "faas.invocation_id")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.as_str()),
                _ => None,
            });
        assert_eq!(inv_attr, Some(invocation_id));

        let init_dur = span
            .attributes
            .iter()
            .find(|kv| kv.key == "faas.init_duration")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::DoubleValue(d)) => Some(*d),
                _ => None,
            });
        assert_eq!(init_dur, Some(150.0));

        // --- Init span ---
        let spans = &decoded.resource_spans[0].scope_spans[0].spans;
        assert_eq!(spans.len(), 2);

        let init_span = &spans[1];
        assert_eq!(init_span.name, "aws.lambda.initialization");
        assert_eq!(init_span.kind, SpanKind::Internal as i32);
        assert_eq!(hex::encode(&init_span.trace_id), trace_id_hex);
        assert_eq!(hex::encode(&init_span.parent_span_id), root_span_id_hex);
        assert_eq!(init_span.start_time_unix_nano, 850_000_000); // (1000 - 150) ms in nanos
        assert_eq!(init_span.end_time_unix_nano, 999_000_000); // (1000 - 1) ms in nanos
        assert!(!init_span.span_id.is_empty());
        assert_ne!(init_span.span_id, span.span_id); // different from root span

        let init_inv_attr = init_span
            .attributes
            .iter()
            .find(|kv| kv.key == "faas.invocation_id")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.as_str()),
                _ => None,
            });
        assert_eq!(init_inv_attr, Some(invocation_id));

        // Init span should NOT have telemetry attributes
        assert!(init_span
            .attributes
            .iter()
            .find(|kv| kv.key == "faas.init_duration")
            .is_none());
    }

    #[test]
    #[serial]
    fn create_overhead_supplementary_span_stores_overhead_span() {
        reset_store();
        let invocation_id = "inv-supp-1";
        let trace_id_hex = "aa".repeat(16);
        let root_span_id_hex = "bb".repeat(8);

        invocation_entry::update(invocation_id, |entry| {
            entry.trace_id = Some(trace_id_hex.clone());
            entry.root_span_id = Some(root_span_id_hex.clone());
            entry.sampled = true;
            entry.start_time = 1_000.0;
            entry.billed_duration = 500.0;
            entry.init_duration = 150.0;
            entry.end_time = 1_100.0;
        });

        create_overhead_supplementary_span(invocation_id);

        let traces = invocation_entry::take_traces_by_id(invocation_id);
        assert_eq!(traces.len(), 1);

        let decoded =
            ExportTraceServiceRequest::decode(traces[0].body.as_slice()).expect("should decode");
        let spans = &decoded.resource_spans[0].scope_spans[0].spans;
        assert_eq!(spans.len(), 1);

        let overhead_span = &spans[0];
        assert_eq!(overhead_span.name, "aws.lambda.overhead");
        assert_eq!(overhead_span.kind, SpanKind::Internal as i32);
        assert_eq!(hex::encode(&overhead_span.trace_id), trace_id_hex);
        assert_eq!(hex::encode(&overhead_span.parent_span_id), root_span_id_hex);
        assert_eq!(overhead_span.start_time_unix_nano, 1_100_000_000);
        assert_eq!(overhead_span.end_time_unix_nano, 1_350_000_000);
        assert!(!overhead_span.span_id.is_empty());
    }

    #[test]
    #[serial]
    fn create_supplementary_spans_noop_without_data() {
        reset_store();
        // No invocation entry exists — should not panic or store anything
        let result = create_supplementary_spans("inv-nonexistent", true);
        assert!(result.is_none());
        let traces = invocation_entry::take_traces_by_id("inv-nonexistent");
        assert!(traces.is_empty());
    }

    fn setup_invocation_entry(invocation_id: &str) {
        let trace_id_hex = "aa".repeat(16);
        let root_span_id_hex = "bb".repeat(8);
        let parent_span_id_hex = "cc".repeat(8);

        invocation_entry::update(invocation_id, |entry| {
            entry.trace_id = Some(trace_id_hex);
            entry.root_span_id = Some(root_span_id_hex);
            entry.parent_span_id = Some(parent_span_id_hex);
            entry.sampled = true;
            entry.start_time = 1_000.0;
            entry.billed_duration = 500.0;
            entry.init_duration = 150.0;
            entry.memory_usage = 256;
            entry.end_time = 1_100.0;
        });
    }

    #[test]
    #[serial]
    fn create_supplementary_spans_skipped_when_xray_enabled() {
        reset_store();
        let invocation_id = "inv-supp-1";
        setup_invocation_entry(invocation_id);

        std::env::set_var("DASH0_XRAY_TRACES_ENABLED", "true");
        std::env::set_var("AWS_LAMBDA_FUNCTION_NAME", "my-function");

        let result = create_supplementary_spans(invocation_id, true);
        assert!(result.is_none());
        let traces = invocation_entry::take_traces_by_id(invocation_id);
        assert!(traces.is_empty());

        std::env::remove_var("DASH0_XRAY_TRACES_ENABLED");
        std::env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
    }

    #[test]
    #[serial]
    fn create_overhead_supplementary_span_skipped_when_xray_enabled() {
        reset_store();
        let invocation_id = "inv-supp-1";
        setup_invocation_entry(invocation_id);

        std::env::set_var("DASH0_XRAY_TRACES_ENABLED", "true");

        create_overhead_supplementary_span(invocation_id);
        let traces = invocation_entry::take_traces_by_id(invocation_id);
        assert!(traces.is_empty());

        std::env::remove_var("DASH0_XRAY_TRACES_ENABLED");
    }
}

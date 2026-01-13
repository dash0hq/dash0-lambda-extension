/// Test data generation utilities for benchmarks

use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, Status};

/// Generate a JSON payload of specified size
pub fn generate_payload(size: usize) -> String {
    let base = "{\"key\":\"";
    let suffix = "\"}";
    let value_size = size.saturating_sub(base.len() + suffix.len());
    let value = "x".repeat(value_size);
    format!("{}{}{}", base, value, suffix)
}

/// Generate a realistic trace with specified number of spans
pub fn generate_trace_request(span_count: usize, invocation_id: &str) -> ExportTraceServiceRequest {
    let mut spans = Vec::with_capacity(span_count);

    for i in 0..span_count {
        spans.push(Span {
            trace_id: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: vec![(i + 1) as u8, 2, 3, 4, 5, 6, 7, 8],
            parent_span_id: if i > 0 { vec![i as u8, 2, 3, 4, 5, 6, 7, 8] } else { vec![] },
            name: format!("test_span_{}", i),
            kind: 1, // SPAN_KIND_INTERNAL
            start_time_unix_nano: 1000000000 + (i as u64 * 1000),
            end_time_unix_nano: 1000001000 + (i as u64 * 1000),
            attributes: vec![
                KeyValue {
                    key: "faas.invocation_id".to_string(),
                    value: Some(AnyValue {
                        value: Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                            invocation_id.to_string(),
                        )),
                    }),
                },
                KeyValue {
                    key: "service.name".to_string(),
                    value: Some(AnyValue {
                        value: Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                            "test-service".to_string(),
                        )),
                    }),
                },
            ],
            status: Some(Status {
                code: 0, // STATUS_CODE_UNSET
                message: String::new(),
            }),
            ..Default::default()
        });
    }

    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans,
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

/// Generate encoded protobuf trace data
pub fn generate_encoded_trace(span_count: usize, invocation_id: &str) -> Vec<u8> {
    use prost::Message;
    let request = generate_trace_request(span_count, invocation_id);
    request.encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_payload_sizes() {
        assert_eq!(generate_payload(100).len(), 100);
        assert_eq!(generate_payload(1024).len(), 1024);
        assert_eq!(generate_payload(10240).len(), 10240);
    }

    #[test]
    fn test_generate_trace() {
        let trace = generate_trace_request(5, "test-id-123");
        assert_eq!(trace.resource_spans.len(), 1);
        assert_eq!(trace.resource_spans[0].scope_spans[0].spans.len(), 5);
    }
}

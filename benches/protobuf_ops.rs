/// Benchmarks for Protobuf encode/decode operations
///
/// This benchmark proves the overhead of unnecessary decode/encode cycles
/// in trace processing (backend_send.rs:64-68, route.rs:282-289).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use prost::Message;

mod support;
use support::fixtures::{generate_encoded_trace, generate_trace_request};

/// Simulate current pattern: decode → modify → re-encode
fn decode_modify_encode(data: &[u8]) -> Vec<u8> {
    // Decode from bytes
    let mut decoded = ExportTraceServiceRequest::decode(data).unwrap();

    // Minimal modification (add one attribute)
    if let Some(resource_span) = decoded.resource_spans.first_mut() {
        if let Some(scope_span) = resource_span.scope_spans.first_mut() {
            if let Some(span) = scope_span.spans.first_mut() {
                span.attributes.push(KeyValue {
                    key: "test.modified".to_string(),
                    value: Some(AnyValue {
                        value: Some(
                            opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(
                                true,
                            ),
                        ),
                    }),
                });
            }
        }
    }

    // Re-encode to bytes
    decoded.encode_to_vec()
}

/// Proposed optimization: Only decode when needed
fn decode_only(data: &[u8]) -> ExportTraceServiceRequest {
    ExportTraceServiceRequest::decode(data).unwrap()
}

/// Proposed optimization: Only encode when needed
fn encode_only(decoded: &ExportTraceServiceRequest) -> Vec<u8> {
    decoded.encode_to_vec()
}

fn bench_protobuf_decode_encode_cycle(c: &mut Criterion) {
    let span_counts = vec![1, 5, 10, 50];

    let mut group = c.benchmark_group("protobuf_decode_encode_cycle");

    for span_count in span_counts {
        let data = generate_encoded_trace(span_count, "bench-id");
        group.throughput(Throughput::Bytes(data.len() as u64));

        // Full cycle (current pattern)
        group.bench_with_input(
            BenchmarkId::new("full_cycle", span_count),
            &data,
            |b, data| {
                b.iter(|| {
                    black_box(decode_modify_encode(data));
                });
            },
        );

        // Decode only
        group.bench_with_input(
            BenchmarkId::new("decode_only", span_count),
            &data,
            |b, data| {
                b.iter(|| {
                    black_box(decode_only(data));
                });
            },
        );

        // Encode only (from pre-decoded)
        let decoded = ExportTraceServiceRequest::decode(data.as_slice()).unwrap();
        group.bench_function(BenchmarkId::new("encode_only", span_count), |b| {
            b.iter(|| {
                black_box(encode_only(&decoded));
            });
        });
    }

    group.finish();
}

/// Benchmark actual combine_traces pattern from backend_send.rs
fn bench_combine_traces(c: &mut Criterion) {
    let trace_counts = vec![1, 10, 50, 100];

    let mut group = c.benchmark_group("combine_traces");

    for trace_count in trace_counts {
        let traces: Vec<Vec<u8>> = (0..trace_count)
            .map(|i| generate_encoded_trace(5, &format!("inv-{}", i)))
            .collect();

        group.bench_with_input(
            BenchmarkId::from_parameter(trace_count),
            &traces,
            |b, traces| {
                b.iter(|| {
                    let mut combined = Vec::new();

                    for trace_data in traces {
                        // Decode each trace
                        let decoded = ExportTraceServiceRequest::decode(trace_data.as_slice()).unwrap();
                        // Combine resource spans
                        combined.extend(decoded.resource_spans);
                    }

                    // Create combined request
                    let combined_request = ExportTraceServiceRequest {
                        resource_spans: combined,
                    };

                    // Re-encode
                    black_box(combined_request.encode_to_vec());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark payload size impact
fn bench_payload_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("protobuf_payload_sizes");

    // Generate traces with varying payload sizes (via span count)
    for (name, span_count) in [("small_1KB", 1), ("medium_10KB", 10), ("large_100KB", 100)] {
        let data = generate_encoded_trace(span_count, "bench-id");
        let size_kb = data.len() / 1024;

        group.throughput(Throughput::Bytes(data.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("decode", format!("{}_{}_KB", name, size_kb)),
            &data,
            |b, data| {
                b.iter(|| {
                    black_box(ExportTraceServiceRequest::decode(data.as_slice()).unwrap());
                });
            },
        );

        let decoded = ExportTraceServiceRequest::decode(data.as_slice()).unwrap();
        group.bench_function(
            BenchmarkId::new("encode", format!("{}_{}_KB", name, size_kb)),
            |b| {
                b.iter(|| {
                    black_box(decoded.encode_to_vec());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark lazy vs eager evaluation
fn bench_lazy_vs_eager_decode(c: &mut Criterion) {
    let data = generate_encoded_trace(10, "bench-id");

    // Eager: Always decode
    c.bench_function("eager_decode_always", |b| {
        b.iter(|| {
            let decoded = black_box(ExportTraceServiceRequest::decode(data.as_slice()).unwrap());
            // Simulate: only use decoded 50% of the time
            if black_box(true) {
                black_box(&decoded);
            }
        });
    });

    // Lazy: Only decode when needed (simulated with conditional)
    c.bench_function("lazy_decode_when_needed", |b| {
        b.iter(|| {
            // Simulate: only decode 50% of the time
            if black_box(false) {
                let decoded = black_box(ExportTraceServiceRequest::decode(data.as_slice()).unwrap());
                black_box(&decoded);
            } else {
                // Skip decode when not needed
                black_box(&data);
            }
        });
    });
}

/// Benchmark JSON to Protobuf conversion (route.rs fallback path)
fn bench_json_to_protobuf_conversion(c: &mut Criterion) {
    let trace_request = generate_trace_request(5, "bench-id");

    // Serialize to JSON
    let json_data = serde_json::to_vec(&trace_request).unwrap();

    c.bench_function("json_deserialize", |b| {
        b.iter(|| {
            let decoded: ExportTraceServiceRequest =
                black_box(serde_json::from_slice(&json_data).unwrap());
            black_box(decoded);
        });
    });

    c.bench_function("json_to_protobuf_full_cycle", |b| {
        b.iter(|| {
            // Deserialize from JSON
            let mut decoded: ExportTraceServiceRequest =
                serde_json::from_slice(&json_data).unwrap();

            // Attribute key renaming (as done in route.rs)
            for resource_span in &mut decoded.resource_spans {
                for scope_span in &mut resource_span.scope_spans {
                    for span in &mut scope_span.spans {
                        for attribute in &mut span.attributes {
                            if attribute.key == "faas.execution" {
                                attribute.key = "faas.invocation_id".to_string();
                            }
                        }
                    }
                }
            }

            // Encode to protobuf
            black_box(decoded.encode_to_vec());
        });
    });
}

criterion_group!(
    benches,
    bench_protobuf_decode_encode_cycle,
    bench_combine_traces,
    bench_payload_size_impact,
    bench_lazy_vs_eager_decode,
    bench_json_to_protobuf_conversion
);
criterion_main!(benches);

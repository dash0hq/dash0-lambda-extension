/// Integration benchmarks comparing all optimization configurations
///
/// This benchmark tests the actual store implementation with different
/// configuration combinations to measure real-world performance impact.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;

mod support;
use support::fixtures::generate_payload;

// Import store functions and config
use aws_lambda_runtime_api_proxy_rs::config::performance::PerformanceConfig;

/// Helper to set config for a benchmark
fn with_config<F, R>(config: PerformanceConfig, f: F) -> R
where
    F: FnOnce() -> R,
{
    unsafe {
        aws_lambda_runtime_api_proxy_rs::config::performance::set_config_override(config);
    }
    let result = f();
    unsafe {
        aws_lambda_runtime_api_proxy_rs::config::performance::clear_config_override();
    }
    result
}

/// Test configurations to benchmark
fn get_test_configs() -> Vec<(&'static str, PerformanceConfig)> {
    vec![
        (
            "baseline",
            PerformanceConfig {
                use_arc_strings: false,
                use_static_http_client: false,
                use_tokio_rwlock: false,
                use_lazy_protobuf: false,
            },
        ),
        (
            "arc_only",
            PerformanceConfig {
                use_arc_strings: true,
                use_static_http_client: false,
                use_tokio_rwlock: false,
                use_lazy_protobuf: false,
            },
        ),
        (
            "rwlock_only",
            PerformanceConfig {
                use_arc_strings: false,
                use_static_http_client: false,
                use_tokio_rwlock: true,
                use_lazy_protobuf: false,
            },
        ),
        (
            "arc_rwlock",
            PerformanceConfig {
                use_arc_strings: true,
                use_static_http_client: false,
                use_tokio_rwlock: true,
                use_lazy_protobuf: false,
            },
        ),
        (
            "all_optimizations",
            PerformanceConfig {
                use_arc_strings: true,
                use_static_http_client: true,
                use_tokio_rwlock: true,
                use_lazy_protobuf: true,
            },
        ),
    ]
}

/// Benchmark store_event_payload and get_event_payload with different configs
fn bench_store_payload_operations(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payload_sizes = vec![("1KB", 1024), ("10KB", 10 * 1024), ("100KB", 100 * 1024)];

    let mut group = c.benchmark_group("store_payload_operations");

    for (size_name, size) in payload_sizes {
        let payload = generate_payload(size);

        for (config_name, config) in get_test_configs() {
            group.bench_with_input(
                BenchmarkId::new(format!("{}_{}", config_name, size_name), size),
                &(&payload, config),
                |b, (payload, config)| {
                    b.iter(|| {
                        with_config(*config, || {
                            runtime.block_on(async {
                                use aws_lambda_runtime_api_proxy_rs::store::{
                                    get_event_payload, store_event_payload,
                                };

                                // Store
                                store_event_payload("bench-id", payload).await;

                                // Get 10 times (realistic access pattern)
                                for _ in 0..10 {
                                    black_box(get_event_payload("bench-id").await);
                                }
                            })
                        })
                    });
                },
            );
        }
    }

    group.finish();
}

/// Benchmark concurrent store access with different configs
fn bench_concurrent_store_access(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payload = generate_payload(10 * 1024); // 10KB

    let mut group = c.benchmark_group("concurrent_store_access");

    for (config_name, config) in get_test_configs() {
        group.bench_with_input(
            BenchmarkId::new("50_tasks", config_name),
            &config,
            |b, config| {
                b.iter(|| {
                    with_config(*config, || {
                        runtime.block_on(async {
                            use aws_lambda_runtime_api_proxy_rs::store::{
                                get_event_payload, store_event_payload,
                            };

                            // Setup: Store payloads
                            for i in 0..10 {
                                store_event_payload(
                                    &format!("concurrent-id-{}", i),
                                    &payload,
                                )
                                .await;
                            }

                            // Concurrent read-heavy workload
                            let handles: Vec<_> = (0..50)
                                .map(|i| {
                                    let id = format!("concurrent-id-{}", i % 10);
                                    tokio::spawn(async move {
                                        for _ in 0..10 {
                                            black_box(get_event_payload(&id).await);
                                        }
                                    })
                                })
                                .collect();

                            for h in handles {
                                h.await.unwrap();
                            }
                        })
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark store_trace operations (tests lazy protobuf)
fn bench_trace_storage(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("trace_storage");

    // Create a sample trace
    use aws_lambda_runtime_api_proxy_rs::store::StoredTrace;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::trace::v1::ResourceSpans;
    use prost::Message;

    let trace_data = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans::default(); 5],
    };
    let encoded = trace_data.encode_to_vec();

    for (config_name, config) in get_test_configs() {
        group.bench_with_input(
            BenchmarkId::new("store_and_retrieve", config_name),
            &config,
            |b, config| {
                b.iter(|| {
                    with_config(*config, || {
                        runtime.block_on(async {
                            use aws_lambda_runtime_api_proxy_rs::store::{
                                store_trace, take_traces,
                            };

                            let trace = StoredTrace::new(
                                hyper::Method::POST,
                                "/v1/traces".to_string(),
                                hyper::HeaderMap::new(),
                                encoded.clone(),
                                vec!["bench-trace-id".to_string()],
                            );

                            store_trace(trace).await;
                            black_box(take_traces().await);
                        })
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark full workflow simulation (realistic scenario)
fn bench_realistic_workflow(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let event_payload = generate_payload(5 * 1024); // 5KB event
    let return_payload = generate_payload(2 * 1024); // 2KB response

    c.bench_function("realistic_workflow_baseline", |b| {
        let config = PerformanceConfig {
            use_arc_strings: false,
            use_static_http_client: false,
            use_tokio_rwlock: false,
            use_lazy_protobuf: false,
        };

        b.iter(|| {
            with_config(config, || {
                runtime.block_on(async {
                    use aws_lambda_runtime_api_proxy_rs::store::{
                        get_event_payload, store_event_payload, store_return_payload,
                        take_return_payload,
                    };

                    // Simulate invocation workflow
                    let inv_id = "workflow-test";

                    // Store event
                    store_event_payload(inv_id, &event_payload).await;

                    // Access event multiple times (span processing)
                    for _ in 0..5 {
                        black_box(get_event_payload(inv_id).await);
                    }

                    // Store return value
                    store_return_payload(inv_id, &return_payload).await;

                    // Retrieve return value
                    black_box(take_return_payload(inv_id).await);
                })
            })
        });
    });

    c.bench_function("realistic_workflow_optimized", |b| {
        let config = PerformanceConfig {
            use_arc_strings: true,
            use_static_http_client: true,
            use_tokio_rwlock: true,
            use_lazy_protobuf: true,
        };

        b.iter(|| {
            with_config(config, || {
                runtime.block_on(async {
                    use aws_lambda_runtime_api_proxy_rs::store::{
                        get_event_payload, store_event_payload, store_return_payload,
                        take_return_payload,
                    };

                    // Simulate invocation workflow
                    let inv_id = "workflow-test-opt";

                    // Store event
                    store_event_payload(inv_id, &event_payload).await;

                    // Access event multiple times (span processing)
                    for _ in 0..5 {
                        black_box(get_event_payload(inv_id).await);
                    }

                    // Store return value
                    store_return_payload(inv_id, &return_payload).await;

                    // Retrieve return value
                    black_box(take_return_payload(inv_id).await);
                })
            })
        });
    });
}

criterion_group!(
    benches,
    bench_store_payload_operations,
    bench_concurrent_store_access,
    bench_trace_storage,
    bench_realistic_workflow
);
criterion_main!(benches);

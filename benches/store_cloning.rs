/// Benchmarks for String cloning vs Arc in store operations
///
/// This benchmark proves the overhead of cloning large String payloads
/// compared to using Arc<String> for zero-cost sharing.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;
use std::sync::Arc;

mod support;
use support::fixtures::generate_payload;

/// Simulate current store implementation with String cloning
fn store_and_get_payload_string(payload: &str) -> String {
    let mut store: HashMap<String, String> = HashMap::new();
    store.insert("test-id".to_string(), payload.to_string());
    // This is what get_event_payload() does - returns cloned String
    store.get("test-id").cloned().unwrap()
}

/// Simulate Arc-based store implementation (proposed optimization)
fn store_and_get_payload_arc(payload: &str) -> Arc<String> {
    let mut store: HashMap<String, Arc<String>> = HashMap::new();
    store.insert("test-id".to_string(), Arc::new(payload.to_string()));
    // Arc clone is just pointer copy - O(1) vs O(n) for String
    store.get("test-id").cloned().unwrap()
}

fn bench_string_vs_arc(c: &mut Criterion) {
    let sizes = vec![
        ("100B", 100),
        ("1KB", 1024),
        ("10KB", 10 * 1024),
        ("100KB", 100 * 1024),
    ];

    let mut group = c.benchmark_group("store_payload_comparison");

    for (name, size) in sizes {
        let payload = generate_payload(size);

        group.throughput(Throughput::Bytes(size as u64));

        // Benchmark current String cloning approach
        group.bench_with_input(
            BenchmarkId::new("string_clone", name),
            &payload,
            |b, payload| {
                b.iter(|| {
                    black_box(store_and_get_payload_string(payload));
                });
            },
        );

        // Benchmark Arc-based approach
        group.bench_with_input(BenchmarkId::new("arc_clone", name), &payload, |b, payload| {
            b.iter(|| {
                black_box(store_and_get_payload_arc(payload));
            });
        });
    }

    group.finish();
}

/// Benchmark concurrent access to demonstrate contention
fn bench_concurrent_string_access(c: &mut Criterion) {
    let payload = generate_payload(10 * 1024); // 10KB payload

    c.bench_function("concurrent_string_clone_10_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let p = payload.clone();
                    std::thread::spawn(move || {
                        for _ in 0..10 {
                            black_box(store_and_get_payload_string(&p));
                        }
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }
        });
    });

    c.bench_function("concurrent_arc_clone_10_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let p = payload.clone();
                    std::thread::spawn(move || {
                        for _ in 0..10 {
                            black_box(store_and_get_payload_arc(&p));
                        }
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }
        });
    });
}

/// Benchmark simulated store pattern with multiple gets
fn bench_multiple_gets_pattern(c: &mut Criterion) {
    let sizes = vec![
        ("100B", 100),
        ("1KB", 1024),
        ("10KB", 10 * 1024),
        ("100KB", 100 * 1024),
    ];

    let mut group = c.benchmark_group("multiple_gets_pattern");

    for (name, size) in sizes {
        let payload = generate_payload(size);
        group.throughput(Throughput::Bytes(size as u64));

        // Benchmark multiple gets with String cloning (current pattern)
        group.bench_with_input(
            BenchmarkId::new("string_10_gets", name),
            &payload,
            |b, payload| {
                b.iter(|| {
                    let stored = store_and_get_payload_string(payload);
                    for _ in 0..9 {
                        // Simulate 10 total accesses
                        let copy = black_box(stored.clone());
                        black_box(copy);
                    }
                });
            },
        );

        // Benchmark multiple gets with Arc (proposed pattern)
        group.bench_with_input(
            BenchmarkId::new("arc_10_gets", name),
            &payload,
            |b, payload| {
                b.iter(|| {
                    let stored = store_and_get_payload_arc(payload);
                    for _ in 0..9 {
                        // Simulate 10 total accesses
                        let copy = black_box(stored.clone());
                        black_box(copy);
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_string_vs_arc,
    bench_concurrent_string_access,
    bench_multiple_gets_pattern
);
criterion_main!(benches);

/// Benchmarks for Mutex contention in async context
///
/// This benchmark proves the performance issues of using parking_lot::Mutex
/// in async code vs tokio::sync::RwLock or DashMap.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use parking_lot::Mutex as ParkingLotMutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;

mod support;
use support::fixtures::generate_payload;

/// Simulate parking_lot::Mutex usage (current implementation)
async fn access_store_parking_lot(
    store: Arc<ParkingLotMutex<HashMap<String, String>>>,
    key: &str,
) -> Option<String> {
    store.lock().get(key).cloned()
}

/// Simulate tokio::sync::RwLock usage (proposed optimization)
async fn access_store_tokio_rwlock(
    store: Arc<TokioRwLock<HashMap<String, String>>>,
    key: &str,
) -> Option<String> {
    store.read().await.get(key).cloned()
}

fn bench_mutex_contention_current(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payload = generate_payload(1024); // 1KB payload

    let mut group = c.benchmark_group("mutex_contention_parking_lot");

    for concurrency in [10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_concurrent", concurrency)),
            &concurrency,
            |b, &concurrency| {
                b.iter(|| {
                    runtime.block_on(async {
                        // Setup store with data
                        let store = Arc::new(ParkingLotMutex::new({
                            let mut map = HashMap::new();
                            map.insert("test-id".to_string(), payload.clone());
                            map
                        }));

                        // Spawn concurrent tasks
                        let handles: Vec<_> = (0..concurrency)
                            .map(|_| {
                                let store = store.clone();
                                tokio::spawn(async move {
                                    for _ in 0..10 {
                                        black_box(access_store_parking_lot(store.clone(), "test-id").await);
                                    }
                                })
                            })
                            .collect();

                        // Wait for all tasks
                        for h in handles {
                            h.await.unwrap();
                        }
                    })
                });
            },
        );
    }

    group.finish();
}

fn bench_mutex_contention_tokio_rwlock(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payload = generate_payload(1024); // 1KB payload

    let mut group = c.benchmark_group("mutex_contention_tokio_rwlock");

    for concurrency in [10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_concurrent", concurrency)),
            &concurrency,
            |b, &concurrency| {
                b.iter(|| {
                    runtime.block_on(async {
                        // Setup store with data
                        let store = Arc::new(TokioRwLock::new({
                            let mut map = HashMap::new();
                            map.insert("test-id".to_string(), payload.clone());
                            map
                        }));

                        // Spawn concurrent tasks
                        let handles: Vec<_> = (0..concurrency)
                            .map(|_| {
                                let store = store.clone();
                                tokio::spawn(async move {
                                    for _ in 0..10 {
                                        black_box(
                                            access_store_tokio_rwlock(store.clone(), "test-id").await,
                                        );
                                    }
                                })
                            })
                            .collect();

                        // Wait for all tasks
                        for h in handles {
                            h.await.unwrap();
                        }
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark simulated store with realistic read-heavy pattern
fn bench_simulated_store_read_heavy(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payload = generate_payload(1024);

    c.bench_function("simulated_store_50_concurrent_reads_parking_lot", |b| {
        b.iter(|| {
            runtime.block_on(async {
                // Setup store with data
                let store = Arc::new(ParkingLotMutex::new({
                    let mut map = HashMap::new();
                    for i in 0..10 {
                        map.insert(format!("id-{}", i), payload.clone());
                    }
                    map
                }));

                // Concurrent reads
                let handles: Vec<_> = (0..50)
                    .map(|i| {
                        let store = store.clone();
                        let id = format!("id-{}", i % 10);
                        tokio::spawn(async move {
                            for _ in 0..10 {
                                black_box(access_store_parking_lot(store.clone(), &id).await);
                            }
                        })
                    })
                    .collect();

                for h in handles {
                    h.await.unwrap();
                }
            })
        });
    });

    c.bench_function("simulated_store_50_concurrent_reads_tokio_rwlock", |b| {
        b.iter(|| {
            runtime.block_on(async {
                // Setup store with data
                let store = Arc::new(TokioRwLock::new({
                    let mut map = HashMap::new();
                    for i in 0..10 {
                        map.insert(format!("id-{}", i), payload.clone());
                    }
                    map
                }));

                // Concurrent reads
                let handles: Vec<_> = (0..50)
                    .map(|i| {
                        let store = store.clone();
                        let id = format!("id-{}", i % 10);
                        tokio::spawn(async move {
                            for _ in 0..10 {
                                black_box(access_store_tokio_rwlock(store.clone(), &id).await);
                            }
                        })
                    })
                    .collect();

                for h in handles {
                    h.await.unwrap();
                }
            })
        });
    });
}

/// Benchmark write-heavy workload (worst case for RwLock)
fn bench_write_heavy_workload(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("write_heavy_workload");

    // Parking lot Mutex (current)
    group.bench_function("parking_lot_mutex", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let store = Arc::new(ParkingLotMutex::new(HashMap::<String, String>::new()));
                let handles: Vec<_> = (0..20)
                    .map(|i| {
                        let store = store.clone();
                        tokio::spawn(async move {
                            for j in 0..50 {
                                let key = format!("key-{}-{}", i, j);
                                store.lock().insert(key.clone(), "value".to_string());
                            }
                        })
                    })
                    .collect();

                for h in handles {
                    h.await.unwrap();
                }
            })
        });
    });

    // Tokio RwLock (proposed - note: writes will be slower)
    group.bench_function("tokio_rwlock", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let store = Arc::new(TokioRwLock::new(HashMap::<String, String>::new()));
                let handles: Vec<_> = (0..20)
                    .map(|i| {
                        let store = store.clone();
                        tokio::spawn(async move {
                            for j in 0..50 {
                                let key = format!("key-{}-{}", i, j);
                                store.write().await.insert(key.clone(), "value".to_string());
                            }
                        })
                    })
                    .collect();

                for h in handles {
                    h.await.unwrap();
                }
            })
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_mutex_contention_current,
    bench_mutex_contention_tokio_rwlock,
    bench_simulated_store_read_heavy,
    bench_write_heavy_workload
);
criterion_main!(benches);

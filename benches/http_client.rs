/// Benchmarks for HTTP client creation vs reuse
///
/// This benchmark proves the significant overhead of creating a new HTTP client
/// on every request (as done in sandbox.rs:53, 80) vs reusing a static client.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use hyper::{Body, Client, Request, Uri};
use once_cell::sync::Lazy;

static REUSABLE_CLIENT: Lazy<Client<hyper::client::HttpConnector>> = Lazy::new(|| Client::new());

/// Simulate creating new client per request (current sandbox.rs pattern)
async fn request_with_new_client() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let uri: Uri = "http://127.0.0.1:9001/test".parse()?;
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())?;

    // Note: This will fail to connect, but we're measuring client creation overhead
    let _ = client.request(req).await;
    Ok(())
}

/// Simulate reusing static client (proposed fix)
async fn request_with_reused_client() -> Result<(), Box<dyn std::error::Error>> {
    let client = &*REUSABLE_CLIENT;
    let uri: Uri = "http://127.0.0.1:9001/test".parse()?;
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())?;

    // Note: This will fail to connect, but we're measuring client reuse benefit
    let _ = client.request(req).await;
    Ok(())
}

fn bench_http_client_creation(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("http_client_new_per_request_single", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let _ = black_box(request_with_new_client().await);
            })
        });
    });

    c.bench_function("http_client_reuse_single", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let _ = black_box(request_with_reused_client().await);
            })
        });
    });
}

fn bench_http_client_multiple_requests(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("http_client_multiple_requests");

    for count in [10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("new_per_request", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    runtime.block_on(async {
                        for _ in 0..count {
                            let _ = black_box(request_with_new_client().await);
                        }
                    })
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("reuse_client", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    runtime.block_on(async {
                        for _ in 0..count {
                            let _ = black_box(request_with_reused_client().await);
                        }
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark just client creation overhead (no request)
fn bench_client_creation_only(c: &mut Criterion) {
    c.bench_function("hyper_client_new", |b| {
        b.iter(|| {
            let _client = black_box(Client::new());
        });
    });

    c.bench_function("lazy_client_access", |b| {
        b.iter(|| {
            let _client = black_box(&*REUSABLE_CLIENT);
        });
    });
}

/// Benchmark cold start initialization
fn bench_lazy_initialization(c: &mut Criterion) {
    c.bench_function("lazy_force_initialization", |b| {
        b.iter_with_setup(
            || {
                // Setup: Create a new Lazy (simulates cold start)
                Lazy::new(|| Client::new())
            },
            |lazy_client| {
                // Measure: Force initialization
                black_box(Lazy::force(&lazy_client));
            },
        );
    });

    // Compare to amortized cost over many uses
    c.bench_function("lazy_amortized_100_accesses", |b| {
        b.iter_with_setup(
            || Lazy::new(|| Client::new()),
            |lazy_client| {
                // Force + 99 accesses
                black_box(Lazy::force(&lazy_client));
                for _ in 0..99 {
                    black_box(&*lazy_client);
                }
            },
        );
    });
}

/// Benchmark concurrent client usage
fn bench_concurrent_client_usage(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("concurrent_new_clients_10_tasks", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let handles: Vec<_> = (0..10)
                    .map(|_| {
                        tokio::spawn(async {
                            for _ in 0..10 {
                                let _ = black_box(request_with_new_client().await);
                            }
                        })
                    })
                    .collect();

                for h in handles {
                    let _ = h.await;
                }
            })
        });
    });

    c.bench_function("concurrent_reused_client_10_tasks", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let handles: Vec<_> = (0..10)
                    .map(|_| {
                        tokio::spawn(async {
                            for _ in 0..10 {
                                let _ = black_box(request_with_reused_client().await);
                            }
                        })
                    })
                    .collect();

                for h in handles {
                    let _ = h.await;
                }
            })
        });
    });
}

criterion_group!(
    benches,
    bench_http_client_creation,
    bench_http_client_multiple_requests,
    bench_client_creation_only,
    bench_lazy_initialization,
    bench_concurrent_client_usage
);
criterion_main!(benches);

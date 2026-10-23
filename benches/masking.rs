//! Cost of extension-side payload masking compared with the existing OTLP pipeline.
//!
//! `cargo bench --bench masking`. Prints allocations per operation first, then runs criterion.
//! The crate is a binary, so the masking modules are included by path; they only use external
//! crates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use flate2::write::GzEncoder;
use flate2::Compression;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message;

// `payload_masking` reaches `masking` as `super::masking`, so both sit at the crate root.
#[allow(dead_code)]
#[path = "../src/otlp/masking.rs"]
mod masking;
#[allow(dead_code)]
#[path = "../src/otlp/payload_masking.rs"]
mod payload_masking;
use payload_masking::{self as pm, PayloadRules, Shape};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs_of(mut f: impl FnMut()) -> usize {
    f(); // warm up lazies
    let before = ALLOCS.load(Ordering::Relaxed);
    f();
    ALLOCS.load(Ordering::Relaxed) - before
}

const SPANS_PER_INVOCATION: usize = 20;

fn kv(key: &str, value: String) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(value)),
        }),
    }
}

fn json_body(target_len: usize, secret: bool) -> String {
    let mut s = String::from(r#"{"orderId":"42","email":"jane@example.com","#);
    if secret {
        s.push_str(r#""password":"hunter2","apiToken":"t-123","#);
    }
    s.push_str(r#""items":["#);
    let mut i = 0;
    while s.len() < target_len.saturating_sub(40) {
        s.push_str(&format!(
            r#"{{"sku":"SKU-{i:05}","qty":{},"note":"gift wrap"}},"#,
            i % 7
        ));
        i += 1;
    }
    s.push_str(r#"{"sku":"last","qty":1}]}"#);
    s
}

/// The payload shapes from the attribute inventory.
fn payloads() -> Vec<(&'static str, String)> {
    let big = json_body(4096, true);
    vec![
        ("small_json", json_body(120, true)),
        ("json_4k", big.clone()),
        ("truncated_json_4k", big[..4000].to_string()),
        (
            "form_encoded",
            "user=bob&password=hunter2&remember=1&csrf_token=abc".to_string(),
        ),
        ("non_matching_json_4k", json_body(4096, false)),
        ("plain_text", "SET session:42 some-opaque-value".to_string()),
    ]
}

/// A Node-like HTTP client span: headers as JSON, bodies, URL with query.
fn http_span(i: usize, body: &str) -> Span {
    Span {
        trace_id: vec![1; 16],
        span_id: vec![(i % 255) as u8; 8],
        name: "POST".to_string(),
        kind: 3,
        start_time_unix_nano: 1,
        end_time_unix_nano: 2,
        attributes: vec![
            kv("http.method", "POST".into()),
            kv("http.status_code", "200".into()),
            kv("net.peer.name", "api.example.com".into()),
            kv("url.full", "https://api.example.com/v1/orders?api_key=k-1&page=2".into()),
            kv(
                "http.request.headers",
                r#"{"content-type":"application/json","authorization":"Bearer abc","x-request-id":"r-1","user-agent":"node"}"#.into(),
            ),
            kv(
                "http.response.headers",
                r#"{"content-type":"application/json","x-amzn-requestid":"a-1","set-cookie":"s=1"}"#.into(),
            ),
            kv("http.request.body", body.to_string()),
            kv("http.response.body", r#"{"ok":true,"sessionToken":"s-1"}"#.into()),
            kv("gen_ai.usage.input_tokens", "12".into()),
        ],
        ..Default::default()
    }
}

fn trace_request(body: &str) -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![
                    kv("service.name", "orders".into()),
                    kv("cloud.provider", "aws".into()),
                ],
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "@opentelemetry/instrumentation-http".into(),
                    ..Default::default()
                }),
                spans: (0..SPANS_PER_INVOCATION)
                    .map(|i| http_span(i, body))
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::new(3));
    e.write_all(body).unwrap();
    e.finish().unwrap()
}

/// What `receiver::traces` does today (minus the span mutations): decode and re-encode.
fn receive(bytes: &[u8]) -> Vec<u8> {
    ExportTraceServiceRequest::decode(bytes)
        .unwrap()
        .encode_to_vec()
}

/// What `exporter::_build_traces_request` + `send_request` do: decode, (mask), encode, gzip.
fn export(stored: &[u8], rules: Option<&PayloadRules>) -> Vec<u8> {
    let mut decoded = ExportTraceServiceRequest::decode(stored).unwrap();
    if let Some(rules) = rules {
        pm::mask_resource_spans(&mut decoded.resource_spans, rules);
    }
    gzip(&decoded.encode_to_vec())
}

fn defaults() -> PayloadRules {
    PayloadRules::from_lookup(|_| None)
}

fn print_allocations() {
    let rules = defaults();
    println!("\nallocations per operation (default rules)");
    println!("{:<24} {:>10} {:>12}", "payload", "scanner", "serde_tree");
    for (name, body) in payloads() {
        let scan = allocs_of(|| {
            black_box(pm::mask_body(black_box(&body), &rules.request_body));
        });
        let tree = allocs_of(|| {
            black_box(masking::mask_json_string(black_box(&body)));
        });
        println!("{name:<24} {scan:>10} {tree:>12}");
    }
    let stored = trace_request(&json_body(4096, true)).encode_to_vec();
    let base = allocs_of(|| {
        black_box(export(&stored, None));
    });
    let masked = allocs_of(|| {
        black_box(export(&stored, Some(&rules)));
    });
    println!(
        "export of {SPANS_PER_INVOCATION} spans (4 KB bodies): {base} allocs baseline, {masked} with masking (+{})\n",
        masked - base
    );
}

fn bench_payloads(c: &mut Criterion) {
    print_allocations();
    let rules = defaults();
    let mut g = c.benchmark_group("mask_payload");
    for (name, body) in payloads() {
        g.throughput(Throughput::Bytes(body.len() as u64));
        g.bench_with_input(BenchmarkId::new("scanner", name), &body, |b, body| {
            b.iter(|| pm::mask_body(black_box(body), &rules.request_body).len())
        });
        // The current extension masking for the Lambda event: parse, mask, serialize.
        g.bench_with_input(BenchmarkId::new("serde_tree", name), &body, |b, body| {
            b.iter(|| masking::mask_json_string(black_box(body)).len())
        });
    }
    g.bench_function("url_query", |b| {
        let url = "https://api.example.com/v1/orders?api_key=k-1&page=2&sort=desc";
        b.iter(|| pm::mask_str(black_box(url), &rules.query_params, Shape::Url).len())
    });
    g.finish();
}

fn bench_pipeline(c: &mut Criterion) {
    let rules = defaults();
    let mut g = c.benchmark_group("pipeline");
    g.throughput(Throughput::Elements(SPANS_PER_INVOCATION as u64));
    for (name, body) in [
        ("small_json", json_body(120, true)),
        ("json_4k", json_body(4096, true)),
    ] {
        let wire = trace_request(&body).encode_to_vec();
        let stored = receive(&wire);
        g.bench_with_input(BenchmarkId::new("receive", name), &wire, |b, w| {
            b.iter(|| receive(black_box(w)).len())
        });
        g.bench_with_input(BenchmarkId::new("export", name), &stored, |b, s| {
            b.iter(|| export(black_box(s), None).len())
        });
        g.bench_with_input(BenchmarkId::new("export+mask", name), &stored, |b, s| {
            b.iter(|| export(black_box(s), Some(&rules)).len())
        });
        let decoded = ExportTraceServiceRequest::decode(stored.as_slice()).unwrap();
        g.bench_with_input(BenchmarkId::new("mask_only", name), &decoded, |b, d| {
            b.iter_batched(
                || d.resource_spans.clone(),
                |mut rs| pm::mask_resource_spans(&mut rs, &rules),
                criterion::BatchSize::SmallInput,
            )
        });
    }
    g.finish();
}

fn bench_logs(c: &mut Criterion) {
    let rules = defaults();
    let record = |i: usize| LogRecord {
        body: Some(AnyValue {
            value: Some(Value::StringValue(if i % 2 == 0 {
                format!(r#"{{"level":"info","msg":"order {i} placed","userId":"u-{i}"}}"#)
            } else {
                format!("order {i} placed for jane@example.com")
            })),
        }),
        attributes: vec![kv("code.function", "handler".into())],
        ..Default::default()
    };
    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: (0..50).map(record).collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let mut g = c.benchmark_group("logs");
    g.throughput(Throughput::Elements(50));
    g.bench_function("mask_50_records", |b| {
        b.iter_batched(
            || request.resource_logs.clone(),
            |mut rl| pm::mask_resource_logs(&mut rl, &rules),
            criterion::BatchSize::SmallInput,
        )
    });
    g.finish();
}

criterion_group!(benches, bench_payloads, bench_pipeline, bench_logs);
criterion_main!(benches);

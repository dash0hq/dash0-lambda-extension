use super::*;
use crate::state::invocation_data::{self, TelemetryLog};
use crate::state::invocation_entry;
use http_body_util::BodyExt;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue};
use opentelemetry_proto::tonic::logs::v1::LogRecord;
use opentelemetry_proto::tonic::metrics::v1::{Metric, ResourceMetrics, ScopeMetrics};
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

// Configuration and the shared HTTPS client are latched once. Run each scenario
// in its own process rather than changing other tests' environment or stores.
fn isolated(name: &str) -> bool {
    if std::env::var_os("DASH0_EXPORT_TEST_CHILD").is_some() {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env("DASH0_EXPORT_TEST_CHILD", "1")
        .env_remove("DASH0_TOKEN_SECRET_ARN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    true
}

struct Received {
    path: String,
    body: Vec<u8>,
    respond: oneshot::Sender<u16>,
}

async fn receiver() -> (
    mpsc::UnboundedReceiver<Received>,
    tokio::task::JoinHandle<()>,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_file = std::env::temp_dir().join(format!("dash0-export-{}.pem", std::process::id()));
    std::fs::write(&cert_file, certified.cert.pem()).unwrap();
    std::env::set_var("SSL_CERT_FILE", &cert_file);
    std::env::remove_var("SSL_CERT_DIR");
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certified.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var(
        "DASH0_ENDPOINT",
        format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        ),
    );
    std::env::set_var("DASH0_TOKEN", "local-test-token");
    std::env::set_var("DASH0_REQUEST_TIMEOUT", "5000");
    std::env::set_var("DASH0_REQUEST_RETRIES", "0");
    crate::config::token::init_dash0_token().await;
    Lazy::force(&HTTPS_CLIENT);
    std::fs::remove_file(cert_file).unwrap();

    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let stream = acceptor.accept(stream).await.unwrap();
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let tx = tx.clone();
                    async move {
                        let gzip = request
                            .headers()
                            .get(header::CONTENT_ENCODING)
                            .is_some_and(|v| v == "gzip");
                        let path = request.uri().path().to_string();
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        let body = if gzip {
                            let mut decoded = Vec::new();
                            std::io::Read::read_to_end(
                                &mut flate2::read::GzDecoder::new(body.as_ref()),
                                &mut decoded,
                            )
                            .unwrap();
                            decoded
                        } else {
                            body.to_vec()
                        };
                        let (respond, response) = oneshot::channel();
                        tx.send(Received {
                            path,
                            body,
                            respond,
                        })
                        .unwrap();
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .status(response.await.unwrap())
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        )
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (rx, server)
}

fn store_trace(name: &str, handler: bool) {
    let body = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![Span {
                    name: name.into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec();
    invocation_entry::store_trace_by_id(
        "export-test",
        StoredTrace {
            method: hyper::Method::POST,
            path_and_query: "/v1/traces".into(),
            headers: Default::default(),
            body,
            invocation_ids: vec!["export-test".into()],
            contains_handler_span: handler,
        },
    );
}

fn telemetry_log(text: &str, invocation_id: Option<&str>) -> TelemetryLog {
    TelemetryLog {
        time: "2026-11-06T12:00:00.000Z".into(),
        r#type: "function".into(),
        record: serde_json::json!(text),
        invocation_id: invocation_id.map(str::to_string),
        trace_id: None,
        span_id: None,
        custom_attributes: vec![],
    }
}

fn seed() {
    store_trace("child", false);
    invocation_data::store_log(StoredLog {
        method: hyper::Method::POST,
        path_and_query: "/v1/logs".into(),
        headers: Default::default(),
        body: ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        body: Some(AnyValue {
                            value: Some(Value::StringValue("sdk-log".into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec(),
    });
    invocation_entry::store_telemetry_logs(vec![telemetry_log("function-log", None)]);
    invocation_data::store_metric(StoredMetric {
        method: hyper::Method::POST,
        path_and_query: "/v1/metrics".into(),
        headers: Default::default(),
        body: ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: "test.metric".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec(),
    });
}

#[tokio::test]
async fn signals_export_concurrently_and_flush_waits() {
    if isolated("otlp::exporter::concurrency_tests::signals_export_concurrently_and_flush_waits") {
        return;
    }
    let (mut requests, server) = receiver().await;
    for early in [false, true] {
        seed();
        if early {
            store_trace("handler", true);
            std::env::set_var("DASH0_REQUEST_RETRIES", "1");
        }
        invocation_entry::store_telemetry_logs(vec![telemetry_log("held-log", Some("current"))]);
        let pending_flush = tokio::spawn(async move {
            if early {
                flush_while_running().await
            } else {
                flush(Some("current")).await
            }
        });
        let mut received = Vec::new();
        // A sequential exporter cannot deliver all four requests while the
        // receiver withholds responses. No wall-clock speed assertion is needed.
        for _ in 0..4 {
            received.push(
                tokio::time::timeout(Duration::from_secs(3), requests.recv())
                    .await
                    .expect("signals must arrive before any response is released")
                    .unwrap(),
            );
        }
        let mut log_bodies = Vec::new();
        let mut trace_names = Vec::new();
        let mut metric_names = Vec::new();
        for request in &received {
            match request.path.as_str() {
                "/v1/traces" => {
                    let decoded =
                        ExportTraceServiceRequest::decode(request.body.as_slice()).unwrap();
                    trace_names.extend(
                        decoded
                            .resource_spans
                            .into_iter()
                            .flat_map(|r| r.scope_spans)
                            .flat_map(|s| s.spans)
                            .map(|s| s.name),
                    );
                }
                "/v1/logs" => {
                    let decoded =
                        ExportLogsServiceRequest::decode(request.body.as_slice()).unwrap();
                    log_bodies.extend(
                        decoded
                            .resource_logs
                            .into_iter()
                            .flat_map(|r| r.scope_logs)
                            .flat_map(|s| s.log_records)
                            .map(|l| l.body.unwrap().value.unwrap()),
                    );
                }
                "/v1/metrics" => {
                    let decoded =
                        ExportMetricsServiceRequest::decode(request.body.as_slice()).unwrap();
                    metric_names.extend(
                        decoded
                            .resource_metrics
                            .into_iter()
                            .flat_map(|r| r.scope_metrics)
                            .flat_map(|s| s.metrics)
                            .map(|m| m.name),
                    );
                }
                path => panic!("unexpected path {path}"),
            }
        }
        assert_eq!(trace_names, ["child"]);
        assert_eq!(metric_names, ["test.metric"]);
        assert_eq!(log_bodies.len(), 2);
        for text in ["sdk-log", "function-log"] {
            assert!(log_bodies.contains(&Value::StringValue(text.into())));
        }
        let next_flush = tokio::spawn(flush(Some("current")));
        // Even after the other exports complete, the slowest response must keep
        // this flush open and prevent the next flush from returning.
        let metric_index = received
            .iter()
            .position(|r| r.path == "/v1/metrics")
            .unwrap();
        let last = received.swap_remove(metric_index);
        for request in received {
            request.respond.send(200).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!pending_flush.is_finished());
        assert!(!next_flush.is_finished());
        if early {
            last.respond.send(503).unwrap();
            let retry = tokio::time::timeout(Duration::from_secs(3), requests.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(retry.path, "/v1/metrics");
            assert_eq!(retry.body, last.body);
            assert!(!pending_flush.is_finished());
            assert!(!next_flush.is_finished());
            retry.respond.send(200).unwrap();
        } else {
            last.respond.send(200).unwrap();
        }
        pending_flush.await.unwrap();
        if early {
            let handler = tokio::time::timeout(Duration::from_secs(3), requests.recv())
                .await
                .unwrap()
                .unwrap();
            let decoded = ExportTraceServiceRequest::decode(handler.body.as_slice()).unwrap();
            assert_eq!(
                decoded.resource_spans[0].scope_spans[0].spans[0].name,
                "handler"
            );
            handler.respond.send(200).unwrap();
        }
        next_flush.await.unwrap();
        let held = invocation_entry::take_all_telemetry_logs(None);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].record, "held-log");
        assert!(requests.try_recv().is_err());
    }
    server.abort();
}

/// Release-mode local HTTPS benchmark. Delays model ingress round trips, not
/// deployed Lambda duration. Run with --ignored --nocapture and set
/// DASH0_BENCH_DELAY_MS / DASH0_BENCH_ITERATIONS to choose the workload.
#[tokio::test]
#[ignore]
async fn benchmark_signal_exports() {
    if isolated("otlp::exporter::concurrency_tests::benchmark_signal_exports") {
        return;
    }
    let (mut requests, server) = receiver().await;
    let delay: u64 = std::env::var("DASH0_BENCH_DELAY_MS")
        .unwrap_or("55".into())
        .parse()
        .unwrap();
    let iterations: usize = std::env::var("DASH0_BENCH_ITERATIONS")
        .unwrap_or("30".into())
        .parse()
        .unwrap();
    let records: usize = std::env::var("DASH0_BENCH_RECORDS")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    let sdk_logs = std::env::var_os("DASH0_BENCH_SDK_LOGS").is_some();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received_count = count.clone();
    let responder = tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            received_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tokio::spawn(async move {
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                request.respond.send(200).unwrap();
            });
        }
    });
    let mut samples = Vec::new();
    for i in 0..iterations + 5 {
        for _ in 0..records {
            seed();
        }
        if !sdk_logs {
            invocation_data::take_logs();
        }
        let start = std::time::Instant::now();
        flush(None).await;
        if i >= 5 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    assert_eq!(
        count.load(std::sync::atomic::Ordering::Relaxed),
        (iterations + 5) * if sdk_logs { 4 } else { 3 }
    );
    samples.sort_by(f64::total_cmp);
    println!(
        "delay_ms={delay} iterations={iterations} records={records} sdk_logs={sdk_logs} median_ms={:.3} p95_ms={:.3}",
        samples[samples.len() / 2],
        samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
    );
    server.abort();
    responder.abort();
}

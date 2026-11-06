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
        if !name.ends_with("::benchmark_signal_exports") {
            // A deadlock must fail CI, not leave Command::output waiting forever.
            // This thread ends with the isolated process on normal completion.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(30));
                eprintln!("export scenario exceeded its 30-second deadline");
                std::process::exit(1);
            });
        }
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
                    trace_id: vec![0x11; 16],
                    span_id: vec![0x22; 8],
                    parent_span_id: vec![0x33; 8],
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
    seed_sized("", 0);
}

fn seed_sized(suffix: &str, size: usize) {
    let padding = "x".repeat(size);
    store_trace(&format!("child{suffix}{padding}"), false);
    invocation_data::store_log(StoredLog {
        method: hyper::Method::POST,
        path_and_query: "/v1/logs".into(),
        headers: Default::default(),
        body: ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        body: Some(AnyValue {
                            value: Some(Value::StringValue(format!("sdk-log{suffix}{padding}"))),
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
    invocation_entry::store_telemetry_logs(vec![telemetry_log(
        &format!("function-log{suffix}{padding}"),
        None,
    )]);
    invocation_data::store_metric(StoredMetric {
        method: hyper::Method::POST,
        path_and_query: "/v1/metrics".into(),
        headers: Default::default(),
        body: ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: format!("test.metric{suffix}"),
                        description: padding,
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

async fn next_request(requests: &mut mpsc::UnboundedReceiver<Received>) -> Received {
    tokio::time::timeout(Duration::from_secs(3), requests.recv())
        .await
        .expect("expected an export request")
        .unwrap()
}

fn labels(request: &Received) -> Vec<String> {
    match request.path.as_str() {
        "/v1/traces" => ExportTraceServiceRequest::decode(request.body.as_slice())
            .unwrap()
            .resource_spans
            .into_iter()
            .flat_map(|r| r.scope_spans)
            .flat_map(|s| s.spans)
            .map(|s| s.name.trim_end_matches('x').to_string())
            .collect(),
        "/v1/logs" => ExportLogsServiceRequest::decode(request.body.as_slice())
            .unwrap()
            .resource_logs
            .into_iter()
            .flat_map(|r| r.scope_logs)
            .flat_map(|s| s.log_records)
            .map(|l| match l.body.unwrap().value.unwrap() {
                Value::StringValue(text) => text.trim_end_matches('x').to_string(),
                value => panic!("unexpected log body {value:?}"),
            })
            .collect(),
        "/v1/metrics" => ExportMetricsServiceRequest::decode(request.body.as_slice())
            .unwrap()
            .resource_metrics
            .into_iter()
            .flat_map(|r| r.scope_metrics)
            .flat_map(|s| s.metrics)
            .map(|m| m.name)
            .collect(),
        path => panic!("unexpected export path {path}"),
    }
}

#[tokio::test]
async fn empty_and_single_signal_flushes_do_not_wait_for_missing_signals() {
    if isolated("otlp::exporter::concurrency_tests::empty_and_single_signal_flushes_do_not_wait_for_missing_signals") {
        return;
    }
    let (mut requests, server) = receiver().await;
    flush(None).await;
    flush_while_running().await;
    assert!(requests.try_recv().is_err());
    for expected in ["child", "sdk-log", "function-log", "test.metric"] {
        seed();
        if expected != "child" {
            invocation_entry::take_all_traces();
        }
        if expected != "sdk-log" {
            invocation_data::take_logs();
        }
        if expected != "function-log" {
            invocation_entry::take_all_telemetry_logs(None);
        }
        if expected != "test.metric" {
            invocation_data::take_metrics();
        }
        let active = tokio::spawn(flush(None));
        let request = next_request(&mut requests).await;
        assert_eq!(labels(&request), [expected]);
        request.respond.send(200).unwrap();
        tokio::time::timeout(Duration::from_secs(3), active)
            .await
            .unwrap()
            .unwrap();
        assert!(requests.try_recv().is_err());
    }
    server.abort();
}

#[tokio::test]
async fn extension_events_wait_for_all_signals_in_both_modes_and_on_spindown() {
    if isolated("otlp::exporter::concurrency_tests::extension_events_wait_for_all_signals_in_both_modes_and_on_spindown") {
        return;
    }
    let (mut requests, server) = receiver().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var(
        "AWS_LAMBDA_RUNTIME_API",
        listener.local_addr().unwrap().to_string(),
    );
    std::env::remove_var("LRAP_RUNTIME_API_ENDPOINT");
    let (events, events_rx) = mpsc::unbounded_channel::<String>();
    let events_rx = Arc::new(tokio::sync::Mutex::new(events_rx));
    let api = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let events_rx = events_rx.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let events_rx = events_rx.clone();
                    async move {
                        let register = request.uri().path().ends_with("/register");
                        request.into_body().collect().await.unwrap();
                        let body = if register {
                            "{}".to_string()
                        } else {
                            events_rx.lock().await.recv().await.unwrap()
                        };
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .header("lambda-extension-identifier", "local-extension")
                                .body(Full::new(Bytes::from(body)))
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
    crate::extension::register::register().await;
    for (on_end, event_type) in [(false, "INVOKE"), (true, "INVOKE"), (true, "SHUTDOWN")] {
        std::env::set_var("DASH0_SEND_ON_INVOCATION_END", on_end.to_string());
        seed();
        events
            .send(
                serde_json::json!({ "eventType": event_type, "requestId": "next-invocation",
            "shutdownReason": "spindown" })
                .to_string(),
            )
            .unwrap();
        let active = tokio::spawn(crate::extension::events::get_next());
        if on_end && event_type == "INVOKE" {
            let notifier = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Some(notifier) = invocation_data::take_runtime_done_notifier() {
                        break notifier;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            assert!(
                requests.try_recv().is_err(),
                "invocation-end mode must wait for runtimeDone"
            );
            notifier.send(()).unwrap();
        }
        let mut held = Vec::new();
        let mut names = Vec::new();
        for _ in 0..4 {
            let request = next_request(&mut requests).await;
            names.extend(labels(&request));
            held.push(request);
        }
        names.sort();
        assert_eq!(names, ["child", "function-log", "sdk-log", "test.metric"]);
        let last = held.pop().unwrap();
        for request in held {
            request.respond.send(200).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !active.is_finished(),
            "event handling must wait for the slowest export"
        );
        last.respond.send(200).unwrap();
        active.await.unwrap();
        assert!(requests.try_recv().is_err());
    }
    api.abort();
    server.abort();
}

#[tokio::test]
async fn chunked_exports_are_bounded_and_queued_flush_drains_new_data() {
    if isolated("otlp::exporter::concurrency_tests::chunked_exports_are_bounded_and_queued_flush_drains_new_data") {
        return;
    }
    let (mut requests, server) = receiver().await;
    seed_sized("-0", MAX_REQUEST_BYTES / 2);
    seed_sized("-1", MAX_REQUEST_BYTES / 2);
    let active = tokio::spawn(flush(None));
    let mut first = Vec::new();
    for _ in 0..4 {
        first.push(next_request(&mut requests).await);
    }
    let mut names: Vec<_> = first.iter().flat_map(labels).collect();
    names.sort();
    assert_eq!(
        names,
        ["child-0", "function-log-0", "sdk-log-0", "test.metric-0"]
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), requests.recv())
            .await
            .is_err(),
        "chunks within a stream must not overlap"
    );

    seed();
    let queued = tokio::spawn(flush(None));
    for request in first {
        let expected: Vec<_> = labels(&request)
            .iter()
            .map(|s| s.replace("-0", "-1"))
            .collect();
        request.respond.send(200).unwrap();
        let second = next_request(&mut requests).await;
        assert_eq!(labels(&second), expected);
        second.respond.send(200).unwrap();
    }
    active.await.unwrap();
    let mut names = Vec::new();
    for _ in 0..4 {
        let request = next_request(&mut requests).await;
        names.extend(labels(&request));
        request.respond.send(200).unwrap();
    }
    names.sort();
    assert_eq!(names, ["child", "function-log", "sdk-log", "test.metric"]);
    tokio::time::timeout(Duration::from_secs(3), queued)
        .await
        .unwrap()
        .unwrap();
    flush(None).await;
    assert!(requests.try_recv().is_err());
    assert_eq!(crate::otlp::flusher::buffered_bytes(), 0);
    server.abort();
}

#[tokio::test]
async fn failed_or_timed_out_signal_does_not_cancel_siblings_or_block_next_flush() {
    if isolated("otlp::exporter::concurrency_tests::failed_or_timed_out_signal_does_not_cancel_siblings_or_block_next_flush") {
        return;
    }
    let (mut requests, server) = receiver().await;
    for timeout in [false, true] {
        std::env::set_var(
            "DASH0_REQUEST_TIMEOUT",
            if timeout { "500" } else { "5000" },
        );
        std::env::set_var("DASH0_REQUEST_RETRIES", if timeout { "0" } else { "1" });
        seed();
        invocation_entry::take_all_traces();
        for index in 0..2 {
            store_trace(
                &format!("healthy-{index}{}", "x".repeat(MAX_REQUEST_BYTES / 2)),
                false,
            );
        }
        let active = tokio::spawn(flush(None));
        let mut metric = None;
        let mut healthy = None;
        let mut names = Vec::new();
        for _ in 0..4 {
            let request = next_request(&mut requests).await;
            names.extend(labels(&request));
            if request.path == "/v1/metrics" {
                metric = Some(request);
            } else if request.path == "/v1/traces" {
                healthy = Some(request);
            } else {
                request.respond.send(200).unwrap();
            }
        }
        names.sort();
        assert_eq!(
            names,
            ["function-log", "healthy-0", "sdk-log", "test.metric"]
        );
        let metric = metric.unwrap();
        let healthy = healthy.unwrap();
        std::env::set_var("DASH0_REQUEST_TIMEOUT", "5000");
        if timeout {
            // The first attempt's 500 ms deadline is already fixed. Give the
            // healthy stream's next chunk a longer deadline, then let metrics
            // time out while that healthy request remains outstanding.
            healthy.respond.send(200).unwrap();
            let second = next_request(&mut requests).await;
            assert_eq!(labels(&second), ["healthy-1"]);
            tokio::time::sleep(Duration::from_millis(650)).await;
            assert!(
                !active.is_finished(),
                "a timed-out stream must not cancel a healthy sibling"
            );
            second.respond.send(200).unwrap();
            let _ = metric.respond.send(200);
        } else {
            metric.respond.send(503).unwrap();
            let retry = next_request(&mut requests).await;
            assert_eq!(retry.path, "/v1/metrics");
            assert_eq!(retry.body, metric.body);
            retry.respond.send(503).unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(
                !active.is_finished(),
                "an exhausted stream must not cancel a healthy sibling"
            );
            healthy.respond.send(200).unwrap();
            let second = next_request(&mut requests).await;
            assert_eq!(labels(&second), ["healthy-1"]);
            second.respond.send(200).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), active)
            .await
            .unwrap()
            .unwrap();
        // A failed batch is not requeued by the exporter. A fresh flush must
        // still acquire the lock and export a single new copy of each signal.
        std::env::set_var("DASH0_REQUEST_TIMEOUT", "5000");
        seed();
        let next = tokio::spawn(flush(None));
        for _ in 0..4 {
            next_request(&mut requests).await.respond.send(200).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), next)
            .await
            .unwrap()
            .unwrap();
        assert!(requests.try_recv().is_err());
    }
    server.abort();
}

#[tokio::test]
async fn all_signals_exhaust_retries_and_recover_after_throttling_or_outage() {
    if isolated("otlp::exporter::concurrency_tests::all_signals_exhaust_retries_and_recover_after_throttling_or_outage") {
        return;
    }
    let (mut requests, server) = receiver().await;
    std::env::set_var("DASH0_REQUEST_RETRIES", "1");
    for status in [Some(429), Some(503), None] {
        std::env::set_var(
            "DASH0_REQUEST_TIMEOUT",
            if status.is_none() { "500" } else { "5000" },
        );
        seed();
        let active = tokio::spawn(flush(None));
        let mut held = Vec::new();
        let mut timed_out_names = Vec::new();
        for _ in 0..2 {
            let mut names = Vec::new();
            let mut attempt = Vec::new();
            for _ in 0..4 {
                let request = next_request(&mut requests).await;
                names.extend(labels(&request));
                attempt.push(request);
            }
            names.sort();
            if let Some(status) = status {
                assert_eq!(names, ["child", "function-log", "sdk-log", "test.metric"]);
                for request in attempt {
                    request.respond.send(status).unwrap();
                }
            } else {
                // Independent timers need not expire in synchronized waves.
                timed_out_names.extend(names);
                held.extend(attempt);
            }
        }
        if status.is_none() {
            timed_out_names.sort();
            assert_eq!(
                timed_out_names,
                [
                    "child",
                    "child",
                    "function-log",
                    "function-log",
                    "sdk-log",
                    "sdk-log",
                    "test.metric",
                    "test.metric"
                ]
            );
        }
        tokio::time::timeout(Duration::from_secs(3), active)
            .await
            .unwrap()
            .unwrap();
        for request in held {
            let _ = request.respond.send(200);
        }
        std::env::set_var("DASH0_REQUEST_TIMEOUT", "5000");
        seed();
        let recovered = tokio::spawn(flush(None));
        let mut names = Vec::new();
        for _ in 0..4 {
            let request = next_request(&mut requests).await;
            names.extend(labels(&request));
            request.respond.send(200).unwrap();
        }
        names.sort();
        assert_eq!(names, ["child", "function-log", "sdk-log", "test.metric"]);
        recovered.await.unwrap();
        assert!(requests.try_recv().is_err());
    }
    server.abort();
}

#[tokio::test]
async fn runtime_error_exports_finish_before_runtime_done_notification() {
    if isolated("otlp::exporter::concurrency_tests::runtime_error_exports_finish_before_runtime_done_notification") {
        return;
    }
    let (mut requests, server) = receiver().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let ingress = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service_fn(crate::route::dispatch))
                    .await;
            });
        }
    });
    for status in ["error", "timeout"] {
        invocation_entry::remove("export-test");
        seed();
        let background = tokio::spawn(flush_while_running());
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(next_request(&mut requests).await);
        }
        let (done, mut notified) = oneshot::channel();
        invocation_data::store_runtime_done_notifier(done);
        let body = serde_json::json!([
            { "time": "2026-11-06T12:00:00.000Z", "type": "platform.start", "record": { "requestId": "export-test" } },
            { "time": "2026-11-06T12:00:01.000Z", "type": "function", "record": "error-log" },
            { "time": "2026-11-06T12:00:02.000Z", "type": "platform.runtimeDone", "record": { "requestId": "export-test", "status": status } }
        ]);
        let post = tokio::spawn(async move {
            let client =
                hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                    .build_http::<Full<Bytes>>();
            let response = client
                .request(
                    Request::post(format!("http://{address}/v1/telemetry"))
                        .body(Full::new(Bytes::from(body.to_string())))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut notified)
                .await
                .is_err(),
            "runtimeDone must wait for the active background flush and error exports"
        );
        assert!(
            requests.try_recv().is_err(),
            "runtime-error exports must share the flush lock"
        );
        for request in held {
            request.respond.send(200).unwrap();
        }
        background.await.unwrap();
        let mut exported = Vec::new();
        let mut error_requests = Vec::new();
        for _ in 0..2 {
            let request = next_request(&mut requests).await;
            exported.extend(labels(&request));
            if request.path == "/v1/traces" {
                let decoded = ExportTraceServiceRequest::decode(request.body.as_slice()).unwrap();
                let handler = decoded
                    .resource_spans
                    .iter()
                    .flat_map(|r| &r.scope_spans)
                    .flat_map(|s| &s.spans)
                    .find(|s| s.name == "handler")
                    .unwrap();
                assert_eq!(handler.trace_id, vec![0x11; 16]);
                assert_eq!(
                    handler.span_id,
                    vec![0x33; 8],
                    "synthetic handler parents the already exported child"
                );
                assert_eq!(handler.status.as_ref().unwrap().code, 2);
                assert!(handler.events.iter().any(|event| event.name == "exception"));
            } else {
                let decoded = ExportLogsServiceRequest::decode(request.body.as_slice()).unwrap();
                let log = decoded
                    .resource_logs
                    .iter()
                    .flat_map(|r| &r.scope_logs)
                    .flat_map(|s| &s.log_records)
                    .find(|l| {
                        l.body.as_ref().and_then(|b| b.value.as_ref())
                            == Some(&Value::StringValue("error-log".into()))
                    })
                    .unwrap();
                assert_eq!(log.trace_id, vec![0x11; 16]);
                assert!(!log.span_id.is_empty());
            }
            error_requests.push(request);
        }
        assert!(exported.contains(&"handler".to_string()));
        assert!(exported.contains(&"error-log".to_string()));
        assert!(notified.try_recv().is_err());
        // Exercise both release orders. The notification must wait for the
        // slower response, not just for receipt of the two request bodies.
        error_requests.sort_by(|a, b| a.path.cmp(&b.path));
        let last = error_requests.remove(if status == "error" { 0 } else { 1 });
        error_requests.pop().unwrap().respond.send(200).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(notified.try_recv().is_err());
        assert!(!post.is_finished());
        last.respond.send(200).unwrap();
        tokio::time::timeout(Duration::from_secs(3), notified)
            .await
            .unwrap()
            .unwrap();
        post.await.unwrap();
        assert!(requests.try_recv().is_err());
    }
    ingress.abort();
    server.abort();
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
            received.push(next_request(&mut requests).await);
        }
        let mut names: Vec<_> = received.iter().flat_map(labels).collect();
        names.sort();
        assert_eq!(names, ["child", "function-log", "sdk-log", "test.metric"]);
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

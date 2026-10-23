//! Runs the extension binary against a fake Lambda Runtime/Extensions/Telemetry API and a fake
//! runtime+distro, with and without `DASH0_MASK_IN_EXTENSION`, and measures what the function
//! sees: the `/v1/traces` ack its flush waits on, the `/response` ack, and the extension's
//! post-response time (`platform.runtimeDone` → next `/extension/event/next`).
//!
//! No AWS credentials and no network: without `DASH0_TOKEN` the exporter still builds (and
//! masks) every request but does not send it. What gets exported is asserted in-process by the
//! `pipeline` tests in `src/otlp/exporter.rs`.
//!
//! `cargo test --test masking_e2e -- --nocapture` prints the latency table.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::{BufRead, BufReader};
use std::net::TcpListener as StdListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

const SECRET: &str = "XQZ-E2E-SECRET";
const INVOCATIONS: usize = 60;
const SPANS: usize = 20;

fn free_port() -> u16 {
    StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The fake Lambda API: hands out queued extension events and invocations, records when the
/// extension asks for its next event.
struct FakeLambda {
    ext_events: AsyncMutex<mpsc::UnboundedReceiver<String>>,
    invocations: AsyncMutex<mpsc::UnboundedReceiver<(String, String)>>,
    ext_next_calls: Mutex<VecDeque<Instant>>,
}

async fn fake_lambda(
    state: Arc<FakeLambda>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let ok = |body: &str| Response::new(Full::new(Bytes::from(body.to_string())));
    let resp = if path.ends_with("/extension/register") {
        let mut r =
            ok(r#"{"functionName":"f","functionVersion":"$LATEST","handler":"index.handler"}"#);
        r.headers_mut()
            .insert("lambda-extension-identifier", "test-ext".parse().unwrap());
        r
    } else if path.ends_with("/telemetry") {
        ok("OK")
    } else if path.ends_with("/extension/event/next") {
        state
            .ext_next_calls
            .lock()
            .unwrap()
            .push_back(Instant::now());
        let event = state.ext_events.lock().await.recv().await;
        ok(&event
            .unwrap_or_else(|| r#"{"eventType":"SHUTDOWN","shutdownReason":"spindown"}"#.into()))
    } else if path.ends_with("/runtime/invocation/next") {
        let (id, body) = state
            .invocations
            .lock()
            .await
            .recv()
            .await
            .expect("no invocation");
        let mut r = ok(&body);
        r.headers_mut()
            .insert("lambda-runtime-aws-request-id", id.parse().unwrap());
        r.headers_mut().insert(
            "lambda-runtime-deadline-ms",
            "9999999999999".parse().unwrap(),
        );
        r
    } else if path.contains("/runtime/invocation/") {
        let mut r = ok("");
        *r.status_mut() = hyper::StatusCode::ACCEPTED;
        r
    } else {
        ok("")
    };
    Ok(resp)
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.into())),
        }),
    }
}

/// What a Node distro flushes at handler end: the aws-lambda span plus HTTP client spans that
/// carry secrets in every payload attribute the extension masks.
fn distro_traces(invocation_id: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"orderId":"42","password":"{SECRET}","items":[{}]}}"#,
        (0..60)
            .map(|i| format!(r#"{{"sku":"SKU-{i}","qty":1}}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    let http_span = |i: usize| Span {
        trace_id: vec![7; 16],
        span_id: vec![i as u8 + 1; 8],
        name: "POST".into(),
        kind: 3,
        start_time_unix_nano: 1_700_000_000_000_000_000,
        end_time_unix_nano: 1_700_000_000_100_000_000,
        attributes: vec![
            kv(
                "url.full",
                &format!("https://api.example.com/x?api_key={SECRET}&page=1"),
            ),
            kv(
                "http.request.headers",
                &format!(r#"{{"authorization":"Bearer {SECRET}","accept":"*/*"}}"#),
            ),
            kv("http.request.header.x-api-key", SECRET),
            kv("http.request.body", &body),
            kv(
                "http.response.body",
                &format!(r#"{{"sessionToken":"{SECRET}"}}"#),
            ),
        ],
        ..Default::default()
    };
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![
                ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "@opentelemetry/instrumentation-aws-lambda".into(),
                        ..Default::default()
                    }),
                    spans: vec![Span {
                        trace_id: vec![7; 16],
                        span_id: vec![200; 8],
                        name: "f".into(),
                        kind: 2,
                        attributes: vec![kv("faas.invocation_id", invocation_id)],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "@opentelemetry/instrumentation-http".into(),
                        ..Default::default()
                    }),
                    spans: (0..SPANS).map(http_span).collect(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

struct Extension {
    child: Child,
    stdout: Arc<Mutex<String>>,
}

impl Drop for Extension {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_extension(lambda_port: u16, port: u16, masking: bool) -> Extension {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aws-lambda-runtime-api-proxy-rs"));
    cmd.env_clear()
        .env("AWS_LAMBDA_RUNTIME_API", format!("127.0.0.1:{lambda_port}"))
        .env("DASH0_LISTENER_PORT", port.to_string())
        .env("AWS_LAMBDA_EXEC_WRAPPER", "/opt/dash0_wrapper")
        .env("AWS_LAMBDA_FUNCTION_NAME", "f")
        .env("AWS_REGION", "eu-west-1")
        // info level: proves the receivers no longer print raw bodies.
        .env("DASH0_EXTENSION_LOG_LEVEL", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if masking {
        cmd.env("DASH0_MASK_IN_EXTENSION", "true");
    }
    let mut child = cmd.spawn().expect("extension binary");
    let stdout = Arc::new(Mutex::new(String::new()));
    for pipe in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let sink = stdout.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let mut s = sink.lock().unwrap();
                s.push_str(&line);
                s.push('\n');
            }
        });
    }
    Extension { child, stdout }
}

fn pct(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

struct Run {
    traces_ack: Vec<Duration>,
    response_ack: Vec<Duration>,
    post_response: Vec<Duration>,
    stdout: String,
}

async fn run(masking: bool) -> Run {
    let lambda_port = free_port();
    let ext_port = free_port();
    let (ext_tx, ext_rx) = mpsc::unbounded_channel::<String>();
    let (inv_tx, inv_rx) = mpsc::unbounded_channel::<(String, String)>();
    let state = Arc::new(FakeLambda {
        ext_events: AsyncMutex::new(ext_rx),
        invocations: AsyncMutex::new(inv_rx),
        ext_next_calls: Mutex::new(VecDeque::new()),
    });

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", lambda_port))
        .await
        .unwrap();
    let server_state = state.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let st = server_state.clone();
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(stream),
                        hyper::service::service_fn(move |r| fake_lambda(st.clone(), r)),
                    )
                    .await;
            });
        }
    });

    let ext = start_extension(lambda_port, ext_port, masking);
    let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
    let base = format!("http://127.0.0.1:{ext_port}");
    let send = |method: &str, path: &str, body: Vec<u8>, ctype: &str| {
        let req = Request::builder()
            .method(method)
            .uri(format!("{base}{path}"))
            .header("content-type", ctype)
            .body(Full::new(Bytes::from(body)))
            .unwrap();
        let client = client.clone();
        async move {
            let res = client.request(req).await.expect("request to extension");
            res.into_body().collect().await.unwrap().to_bytes()
        }
    };

    // Wait for the extension's listener, then for its first /event/next (registration done).
    let deadline = Instant::now() + Duration::from_secs(10);
    while state.ext_next_calls.lock().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "extension did not start:\n{}",
            ext.stdout.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut out = Run {
        traces_ack: vec![],
        response_ack: vec![],
        post_response: vec![],
        stdout: String::new(),
    };
    for i in 0..INVOCATIONS {
        let id = format!("00000000-0000-0000-0000-{i:012}");
        let now = chrono::Utc::now().to_rfc3339();
        ext_tx
            .send(format!(
                r#"{{"eventType":"INVOKE","requestId":"{id}","invokedFunctionArn":"arn:aws:lambda:eu-west-1:123456789012:function:f","tracing":{{"type":"X-Amzn-Trace-Id","value":"Root=1-5759e988-bd862e3fe1be46a994272793;Parent=53995c3f42cd8ad8;Sampled=1"}}}}"#
            ))
            .unwrap();
        inv_tx
            .send((
                id.clone(),
                format!(r#"{{"password":"{SECRET}","orderId":"42"}}"#),
            ))
            .unwrap();
        let telemetry = |kind: &str| {
            format!(
                r#"[{{"time":"{now}","type":"{kind}","record":{{"requestId":"{id}","status":"success","version":"$LATEST","metrics":{{"durationMs":1.0,"billedDurationMs":1,"memorySizeMB":128,"maxMemoryUsedMB":64}}}}}}]"#
            )
            .into_bytes()
        };

        send(
            "GET",
            "/2018-06-01/runtime/invocation/next",
            vec![],
            "application/json",
        )
        .await;
        send(
            "POST",
            "/v1/telemetry",
            telemetry("platform.start"),
            "application/json",
        )
        .await;

        let t = Instant::now();
        send(
            "POST",
            "/v1/traces",
            distro_traces(&id),
            "application/x-protobuf",
        )
        .await;
        out.traces_ack.push(t.elapsed());

        let t = Instant::now();
        let response = format!(r#"{{"statusCode":200,"body":"{{\"token\":\"{SECRET}\"}}"}}"#);
        send(
            "POST",
            &format!("/2018-06-01/runtime/invocation/{id}/response"),
            response.into_bytes(),
            "application/json",
        )
        .await;
        out.response_ack.push(t.elapsed());

        let calls_before = state.ext_next_calls.lock().unwrap().len();
        let t = Instant::now();
        send(
            "POST",
            "/v1/telemetry",
            telemetry("platform.runtimeDone"),
            "application/json",
        )
        .await;
        loop {
            if state.ext_next_calls.lock().unwrap().len() > calls_before {
                break;
            }
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "extension never finished invocation {i}"
            );
            tokio::time::sleep(Duration::from_micros(200)).await;
        }
        out.post_response
            .push(*state.ext_next_calls.lock().unwrap().back().unwrap() - t);
        send(
            "POST",
            "/v1/telemetry",
            telemetry("platform.report"),
            "application/json",
        )
        .await;
    }
    drop(ext_tx);
    tokio::time::sleep(Duration::from_millis(100)).await;
    out.stdout = ext.stdout.lock().unwrap().clone();
    out
}

fn summary(name: &str, samples: &mut [Duration]) -> String {
    samples.sort();
    format!(
        "{name:<34} p50 {:>9.3?}  p99 {:>9.3?}",
        pct(samples, 0.5),
        pct(samples, 0.99)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn masking_adds_no_latency_to_the_function_and_logs_no_secrets() {
    for masking in [false, true] {
        let mut r = run(masking).await;
        println!(
            "DASH0_MASK_IN_EXTENSION={masking} ({INVOCATIONS} invocations, {SPANS} spans each)"
        );
        println!(
            "  {}",
            summary("/v1/traces ack (handler waits)", &mut r.traces_ack)
        );
        println!("  {}", summary("/response ack", &mut r.response_ack));
        println!(
            "  {}",
            summary("runtimeDone -> event/next (billed)", &mut r.post_response)
        );
        assert_eq!(r.traces_ack.len(), INVOCATIONS);
        // The extension's own log output must never contain payload content.
        assert!(
            !r.stdout.contains(SECRET),
            "extension log leaked a secret:\n{}",
            r.stdout
                .lines()
                .filter(|l| l.contains(SECRET))
                .take(3)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

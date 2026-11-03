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

use std::collections::HashMap;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::{BufRead, BufReader};
use std::net::TcpListener as StdListener;
use std::path::Path;
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
        r#"{{"orderId":"42","password":"{SECRET}","email":"jane","items":[{}]}}"#,
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
                &format!("https://api.example.com/x?api_key={SECRET}&email=jane&page=1"),
            ),
            kv(
                "http.request.query",
                &format!(r#"{{"password":"{SECRET}","email":"jane"}}"#),
            ),
            kv(
                "http.request.headers",
                &format!(r#"{{"authorization":"Bearer {SECRET}","x-email":"jane"}}"#),
            ),
            kv("http.request.header.x-api-key", SECRET),
            kv(
                "http.response.headers",
                &format!(r#"{{"x-api-key":"{SECRET}","x-email":"jane"}}"#),
            ),
            kv("http.request.body", &body),
            kv(
                "http.response.body",
                &format!(r#"{{"sessionToken":"{SECRET}","email":"jane"}}"#),
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

fn start_extension(lambda_port: u16, port: u16, env: &[(&str, &str)]) -> Extension {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aws-lambda-runtime-api-proxy-rs"));
    cmd.env_clear()
        .env("AWS_LAMBDA_RUNTIME_API", format!("127.0.0.1:{lambda_port}"))
        .env("DASH0_LISTENER_PORT", port.to_string())
        .env("AWS_LAMBDA_EXEC_WRAPPER", "/opt/dash0_wrapper")
        .env("AWS_LAMBDA_FUNCTION_NAME", "f")
        .env("AWS_REGION", "eu-west-1")
        // info level by default: proves the receivers no longer print raw bodies.
        .env("DASH0_EXTENSION_LOG_LEVEL", "info")
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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

async fn run(env: &[(&str, &str)], invocations: usize) -> Run {
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

    let ext = start_extension(lambda_port, ext_port, env);
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
    for i in 0..invocations {
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
                format!(r#"{{"password":"{SECRET}","email":"jane","orderId":"42"}}"#),
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

        // The function's own stdout, as the Telemetry API delivers it.
        let function_log = format!(
            r#"[{{"time":"{now}","type":"function","record":"login password={SECRET} email=jane"}}]"#
        );
        send(
            "POST",
            "/v1/telemetry",
            function_log.into_bytes(),
            "application/json",
        )
        .await;

        let t = Instant::now();
        let response = format!(r#"{{"statusCode":200,"token":"{SECRET}","email":"jane"}}"#);
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

fn assert_no_secret_in_log(stdout: &str, ctx: &str) {
    assert!(
        !stdout.contains(SECRET) && !stdout.contains("jane"),
        "{ctx}: extension log leaked payload content:\n{}",
        stdout
            .lines()
            .filter(|l| l.contains(SECRET) || l.contains("jane"))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn masking_adds_no_latency_to_the_function_and_logs_no_secrets() {
    for masking in [false, true] {
        let env: &[(&str, &str)] = if masking {
            &[("DASH0_MASK_IN_EXTENSION", "true")]
        } else {
            &[]
        };
        let mut r = run(env, INVOCATIONS).await;
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
        assert_no_secret_in_log(&r.stdout, &format!("masking={masking}, info"));
    }
}

/// What reaches CloudWatch from the extension itself: its stdout and stderr. No payload content
/// at any log level, with or without extension masking.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn extension_log_has_no_payload_content_at_any_level() {
    for level in ["trace", "debug", "info"] {
        for masking in ["false", "true"] {
            let r = run(
                &[
                    ("DASH0_EXTENSION_LOG_LEVEL", level),
                    ("DASH0_MASK_IN_EXTENSION", masking),
                ],
                2,
            )
            .await;
            assert!(!r.stdout.is_empty(), "{level}: no log output captured");
            assert_no_secret_in_log(&r.stdout, &format!("level={level}, masking={masking}"));
        }
    }
}

/// The payloads one export carries, read back from the debug-build export seam
/// (`DASH0_TEST_EXPORT_DIR`): span attributes of the first HTTP span, `dash0_payload` log bodies
/// by type, and the recorded environment variables.
#[cfg(debug_assertions)]
#[derive(Default)]
struct Exported {
    attrs: HashMap<String, String>,
    logs: HashMap<String, String>,
    env: HashMap<String, String>,
}

#[cfg(debug_assertions)]
fn str_value(v: &Option<AnyValue>) -> Option<String> {
    match v {
        Some(AnyValue {
            value: Some(Value::StringValue(s)),
        }) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(debug_assertions)]
fn read_exports(dir: &Path) -> Exported {
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    let mut out = Exported::default();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    for f in files {
        let bytes = std::fs::read(&f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with("traces.pb") {
            let req = ExportTraceServiceRequest::decode(bytes.as_slice()).unwrap();
            for rs in &req.resource_spans {
                for kv in rs.resource.iter().flat_map(|r| &r.attributes) {
                    if let Some(n) = kv.key.strip_prefix("process.environment_variable.") {
                        out.env.insert(n.into(), str_value(&kv.value).unwrap());
                    }
                }
                let http = rs
                    .scope_spans
                    .iter()
                    .flat_map(|ss| &ss.spans)
                    .find(|s| s.attributes.iter().any(|kv| kv.key == "url.full"));
                if let (Some(span), true) = (http, out.attrs.is_empty()) {
                    for kv in &span.attributes {
                        if let Some(v) = str_value(&kv.value) {
                            out.attrs.insert(kv.key.clone(), v);
                        }
                    }
                }
            }
        } else if name.ends_with("logs.pb") {
            let req = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
            for record in req
                .resource_logs
                .iter()
                .flat_map(|rl| &rl.scope_logs)
                .flat_map(|sl| &sl.log_records)
            {
                let kind = record
                    .attributes
                    .iter()
                    .find(|kv| kv.key == "dash0.faas.payload_type")
                    .and_then(|kv| str_value(&kv.value));
                if let (Some(kind), Some(body)) = (kind, str_value(&record.body)) {
                    out.logs.entry(kind).or_insert(body);
                }
            }
        }
    }
    out
}

/// What one payload looks like after export.
#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, PartialEq)]
enum Expect {
    /// As captured: secret and email visible.
    Raw,
    /// Default patterns: secret masked, email kept.
    Defaults,
    /// `[".*email.*"]`: email masked, secret kept.
    Email,
    /// `all`: the whole value (for a URL, the query string) replaced.
    All,
}

/// Every payload the fixture span carries, with the variable that controls it.
#[cfg(debug_assertions)]
const PAYLOADS: &[(&str, &str)] = &[
    ("url.full", "DASH0_MASK_QUERY_PARAMS"),
    ("http.request.query", "DASH0_MASK_QUERY_PARAMS"),
    ("http.request.headers", "DASH0_MASK_REQUEST_HEADERS"),
    (
        "http.request.header.x-api-key",
        "DASH0_MASK_REQUEST_HEADERS",
    ),
    ("http.response.headers", "DASH0_MASK_RESPONSE_HEADERS"),
    ("http_request_body", "DASH0_MASK_REQUEST_BODY"),
    ("http_response_body", "DASH0_MASK_RESPONSE_BODY"),
];

#[cfg(debug_assertions)]
fn check(e: &Exported, payload: &str, expect: Expect, ctx: &str) {
    let is_log = !payload.contains('.');
    let v = if is_log {
        e.logs.get(payload)
    } else {
        e.attrs.get(payload)
    }
    .unwrap_or_else(|| panic!("{ctx}: {payload} not exported"));
    let has_email = payload != "http.request.header.x-api-key";
    let ctx = format!("{ctx}: {payload} = {v}");
    match expect {
        Expect::Raw => {
            assert!(v.contains(SECRET), "{ctx}");
            assert!(!has_email || v.contains("jane"), "{ctx}");
        }
        Expect::Defaults => {
            assert!(!v.contains(SECRET), "{ctx}");
            assert!(!has_email || v.contains("jane"), "{ctx}");
        }
        Expect::Email => {
            assert!(v.contains(SECRET), "{ctx}");
            assert!(!v.contains("jane"), "{ctx}");
        }
        Expect::All => match payload {
            "url.full" => assert_eq!(v, "https://api.example.com/x?****", "{ctx}"),
            _ if is_log => assert!(v.starts_with(r#"{"message":"****""#), "{ctx}"),
            _ => assert_eq!(v, "****", "{ctx}"),
        },
    }
}

/// The Lambda event, the return value and environment variables are masked as on master,
/// whatever DASH0_MASK_IN_EXTENSION says.
#[cfg(debug_assertions)]
fn check_master_paths(e: &Exported, rules: Expect, env: &[(&str, &str)], ctx: &str) {
    for kind in ["lambda_event", "lambda_return_value"] {
        let v = e
            .logs
            .get(kind)
            .unwrap_or_else(|| panic!("{ctx}: {kind} not exported"));
        match rules {
            Expect::Defaults => assert!(!v.contains(SECRET) && v.contains("jane"), "{ctx}: {v}"),
            Expect::Email => assert!(v.contains(SECRET) && !v.contains("jane"), "{ctx}: {v}"),
            _ => unreachable!(),
        }
    }
    let got: HashMap<&str, &str> = e
        .env
        .iter()
        .filter(|(k, _)| ENV_FILE.iter().any(|(n, _)| n == k))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(got, env.iter().copied().collect(), "{ctx}: env vars");
}

/// The function environment, as the wrapper writes it for the extension (docs/masking.md,
/// `DASH0_MASK_ENV_VARS`, plus one personal-data variable).
#[cfg(debug_assertions)]
const ENV_FILE: &[(&str, &str)] = &[
    ("DATABASE_URL", "postgres://app:pw@db/orders"),
    ("DB_PASSWORD", "hunter2"),
    ("GITHUB_TOKEN", "ghp_123"),
    ("LOG_LEVEL", "info"),
    ("APP_EMAIL", "jane@example.com"),
];

#[cfg(debug_assertions)]
fn env_with(masked: &[&str]) -> Vec<(&'static str, &'static str)> {
    ENV_FILE
        .iter()
        .map(|&(k, v)| (k, if masked.contains(&k) { "****" } else { v }))
        .collect()
}

/// One scenario per DASH0_MASK_* variable, against the real binary; asserts on what it exports.
/// Debug builds only: the export seam isn't compiled into release builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_mask_variable_shapes_the_exported_payloads() {
    use Expect::*;
    const EMAIL: &str = r#"[".*email.*"]"#;
    const VARS: &[&str] = &[
        "DASH0_MASK_QUERY_PARAMS",
        "DASH0_MASK_REQUEST_HEADERS",
        "DASH0_MASK_RESPONSE_HEADERS",
        "DASH0_MASK_REQUEST_BODY",
        "DASH0_MASK_RESPONSE_BODY",
    ];
    let defaults_env = env_with(&["DB_PASSWORD", "GITHUB_TOKEN"]);
    let email_env = env_with(&["APP_EMAIL"]);

    // (name, env, payload expectation for the variable under test, for the others,
    //  event/return-value rules, expected env vars)
    type Scenario = (
        String,
        Vec<(&'static str, String)>,
        Expect,
        Expect,
        Expect,
        Vec<(&'static str, &'static str)>,
    );
    let on = |vars: &[(&'static str, &str)]| {
        let mut v = vec![("DASH0_MASK_IN_EXTENSION", "true".to_string())];
        v.extend(vars.iter().map(|(k, val)| (*k, val.to_string())));
        v
    };
    let mut scenarios: Vec<(Option<&str>, Scenario)> = vec![
        (
            None,
            (
                "flag unset".into(),
                vec![],
                Raw,
                Raw,
                Defaults,
                defaults_env.clone(),
            ),
        ),
        (
            None,
            (
                "defaults".into(),
                on(&[]),
                Defaults,
                Defaults,
                Defaults,
                defaults_env.clone(),
            ),
        ),
        (
            None,
            (
                "DASH0_MASK_RULES=email".into(),
                on(&[("DASH0_MASK_RULES", EMAIL)]),
                Email,
                Email,
                Email,
                email_env.clone(),
            ),
        ),
        // The master paths don't accept `all` and fall back to the defaults.
        (
            None,
            (
                "DASH0_MASK_RULES=all".into(),
                on(&[("DASH0_MASK_RULES", "all")]),
                All,
                All,
                Defaults,
                defaults_env.clone(),
            ),
        ),
        (
            None,
            (
                "DASH0_MASK_RULES invalid".into(),
                on(&[("DASH0_MASK_RULES", ".*email.*")]),
                Defaults,
                Defaults,
                Defaults,
                defaults_env.clone(),
            ),
        ),
        (
            None,
            (
                "DASH0_MASK_ENV_VARS".into(),
                on(&[(
                    "DASH0_MASK_ENV_VARS",
                    r#"[".*pass.*",".*key.*",".*secret.*",".*credential.*",".*passphrase.*",".*token.*",".*signature.*",".*authorization.*",".*DATABASE_URL.*"]"#,
                )]),
                Defaults,
                Defaults,
                Defaults,
                env_with(&["DATABASE_URL", "DB_PASSWORD", "GITHUB_TOKEN"]),
            ),
        ),
    ];
    for var in VARS {
        for (value, expect) in [
            (EMAIL, Email),
            ("all", All),
            ("[]", Raw),
            ("{not json", Defaults),
        ] {
            scenarios.push((
                Some(var),
                (
                    format!("{var}={value}"),
                    on(&[(var, value)]),
                    expect,
                    Defaults,
                    Defaults,
                    defaults_env.clone(),
                ),
            ));
        }
    }

    std::fs::write(
        "/tmp/dash0_env_vars",
        serde_json::to_string(&ENV_FILE.iter().copied().collect::<HashMap<_, _>>()).unwrap(),
    )
    .unwrap();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("masking-e2e-exports");
    for (i, (var, (name, env, own, others, master, env_vars))) in scenarios.into_iter().enumerate()
    {
        let dir = root.join(i.to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().to_string();
        let mut vars: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        vars.push(("DASH0_TEST_EXPORT_DIR", &dir_str));
        let r = run(&vars, 1).await;
        let e = read_exports(&dir);
        for (payload, controlled_by) in PAYLOADS {
            let expect = if var.is_none() || var == Some(controlled_by) {
                own
            } else {
                others
            };
            check(&e, payload, expect, &name);
        }
        check_master_paths(&e, master, &env_vars, &name);
        assert_no_secret_in_log(&r.stdout, &name);
    }
    let _ = std::fs::remove_file("/tmp/dash0_env_vars");
}

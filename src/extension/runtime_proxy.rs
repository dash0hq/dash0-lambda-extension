use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use once_cell::sync::Lazy;

use hyper::HeaderMap;

use crate::config::endpoints;
use crate::config::is_auto_instrumented_disabled;
use crate::otlp::log_mutations::build_payload_log;
use crate::otlp::span_mutations::{
    apply_return_value_error_to_stored_traces, build_synthetic_trace,
};
use crate::route::{empty_body, full_body, streaming_body, ReqBody, ResBody};
use crate::state::invocation_data::store_current_invocation_id;
use crate::state::invocation_entry;
use crate::util::parsers::extract_invocation_id_from_path;
use crate::util::truncate::process_payload;

static HTTP_CLIENT: Lazy<Client<HttpConnector, ReqBody>> =
    Lazy::new(|| Client::builder(TokioExecutor::new()).build_http());

fn req_empty() -> ReqBody {
    Full::new(Bytes::new())
}

fn req_from_bytes(bytes: Bytes) -> ReqBody {
    Full::new(bytes)
}

pub async fn next(
    headers: &HeaderMap,
    path: &str,
) -> Result<(Arc<String>, Response<Incoming>), hyper_util::client::legacy::Error> {
    let uri = match hyper::Uri::builder()
        .scheme("http")
        .authority(endpoints::sandbox_runtime_api())
        .path_and_query(path)
        .build()
    {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!(
                "[{}] Error building Sandbox Lambda Runtime API endpoint URL: {}",
                crate::log_prefix(),
                e
            );
            panic!(
                "[{}] Failed to build Runtime API URI - severe misconfiguration: {}",
                crate::log_prefix(),
                e
            );
        }
    };

    let mut req = match Request::builder().method("GET").uri(uri).body(req_empty()) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!(
                "[{}] Cannot create Sandbox Lambda Runtime API request: {}",
                crate::log_prefix(),
                e
            );
            panic!(
                "[{}] Failed to build Runtime API request - severe misconfiguration: {}",
                crate::log_prefix(),
                e
            );
        }
    };

    *req.headers_mut() = headers.clone();

    let response = HTTP_CLIENT.request(req).await?;

    match response.headers().get("lambda-runtime-aws-request-id") {
        Some(id) => match id.to_str() {
            Ok(id_str) => Ok((Arc::new(id_str.to_string()), response)),
            Err(e) => {
                tracing::error!(
                    "[{}] Error parsing Lambda Runtime API request ID: {}",
                    crate::log_prefix(),
                    e
                );
                panic!(
                    "[{}] Invalid request ID header from Lambda Runtime API: {}",
                    crate::log_prefix(),
                    e
                );
            }
        },
        None => {
            tracing::error!("[{}] Sandbox Lambda Runtime API response missing 'lambda-runtime-aws-request-id' header", crate::log_prefix());
            panic!("[{}] Lambda Runtime API response missing required header - this should never happen", crate::log_prefix());
        }
    }
}

/// Pass-through the request, but log the unhandled path and method
#[allow(dead_code)]
pub async fn notfound_passthru_proxy(
    req: Request<Incoming>,
) -> Result<Response<ResBody>, hyper::Error> {
    tracing::info!(
        "[{}] Route not found: path={} method={}",
        crate::log_prefix(),
        &req.uri().path(),
        &req.method()
    );
    passthru_proxy(req).await
}

#[allow(dead_code)]
pub async fn passthru_proxy(req: Request<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
    let (parts, body) = req.into_parts();
    let body_bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            tracing::error!(
                "[{}] Failed to read request body in passthru_proxy: {}",
                crate::log_prefix(),
                e
            );
            return Ok(Response::builder()
                .status(500)
                .body(full_body(Bytes::from_static(
                    b"500 - Internal Error: Failed to read request body",
                )))
                .unwrap_or_else(|_| Response::new(empty_body())));
        }
    };

    forward_to_runtime_api(parts, body_bytes).await
}

fn stream_response(res: Response<Incoming>) -> Response<ResBody> {
    let (parts, body) = res.into_parts();
    Response::from_parts(parts, streaming_body(body))
}

pub async fn proxy_invocation_next(
    req: Request<Incoming>,
) -> Result<Response<ResBody>, hyper::Error> {
    'getNext: loop {
        // track either initialization  -or-
        // how long it took to process the event and request next
        //
        crate::stats::get_next_event();
        crate::state::global::init_env_var_attrs();

        let (aws_request_id, response) = match next(req.headers(), req.uri().path()).await {
            Err(e) => {
                tracing::error!(
                    "[{}]  Error getting next invocation from Runtime API: {}",
                    crate::log_prefix(),
                    e
                );
                tracing::trace!("[{}] uri: {}", crate::log_prefix(), req.uri());
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue 'getNext;
            }
            Ok(response) => response,
        };

        // start the counter on the new event
        crate::stats::event_start();

        store_current_invocation_id(aws_request_id.as_str());

        tracing::info!(
            "[{}] Got invocation next: {}",
            crate::log_prefix(),
            aws_request_id.as_str()
        );

        match validate_and_mangle_next_event(aws_request_id, response).await {
            Ok(response) => {
                return Ok(response);
            }
            Err(req) => {
                let _ = HTTP_CLIENT.request(req).await;
                continue 'getNext;
            }
        }
    }
}

pub async fn invocation_response_proxy(
    req: Request<Incoming>,
) -> Result<Response<ResBody>, hyper::Error> {
    let start = Instant::now();
    let invocation_id = extract_invocation_id_from_path(req.uri().path());
    let (parts, body) = req.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    let return_payload = process_payload(&String::from_utf8_lossy(&body_bytes));
    let res = forward_to_runtime_api(parts, body_bytes).await;
    if let Some(id) = invocation_id {
        if let Some(log) = build_payload_log(
            &return_payload,
            "lambda_return_value",
            &id,
            None,
            None,
            None,
        ) {
            invocation_entry::update(&id, |entry| {
                entry.logs.push(log);
            });
        }

        if is_auto_instrumented_disabled() {
            if let Some(trace) =
                build_synthetic_trace(&id, None, Some(return_payload.as_str()), &Vec::new())
            {
                invocation_entry::store_trace_by_id(&id, trace);
            }
        } else {
            apply_return_value_error_to_stored_traces(&id, &return_payload);
        }
    }
    tracing::info!(
        "[{}] Total handle time for invocation response: {} ms",
        crate::log_prefix(),
        start.elapsed().as_millis()
    );
    res
}

/// Forward a request whose body has already been buffered to bytes to the
/// sandbox runtime API. Shared by `passthru_proxy` (which collects the
/// `Incoming` body itself) and `invocation_response_proxy` (which needs the
/// body buffered for masking before forwarding).
async fn forward_to_runtime_api(
    parts: hyper::http::request::Parts,
    body_bytes: Bytes,
) -> Result<Response<ResBody>, hyper::Error> {
    let start = Instant::now();
    let endpoint_uri: Uri = match Uri::builder()
        .scheme("http")
        .authority(endpoints::sandbox_runtime_api())
        .path_and_query(parts.uri.path())
        .build()
    {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!(
                "[{}] Failed to build URI for sandbox runtime API: {}",
                crate::log_prefix(),
                e
            );
            return Ok(Response::builder()
                .status(502)
                .body(full_body(Bytes::from_static(
                    b"502 - Bad Gateway: Invalid runtime API configuration",
                )))
                .unwrap_or_else(|_| Response::new(empty_body())));
        }
    };

    let mut endpoint_req: Request<ReqBody> = Request::from_parts(parts, req_from_bytes(body_bytes));
    *endpoint_req.uri_mut() = endpoint_uri.clone();

    let method = endpoint_req.method().clone();

    match HTTP_CLIENT.request(endpoint_req).await {
        Ok(res) => {
            tracing::info!(
                "[{}] passthru_proxy - {} {} completed in {} ms",
                crate::log_prefix(),
                method,
                endpoint_uri,
                start.elapsed().as_millis()
            );
            Ok(stream_response(res))
        }
        Err(e) => {
            tracing::error!(
                "[{}] Error invoking endpoint ({} on {}): {:?}",
                crate::log_prefix(),
                method,
                endpoint_uri,
                e
            );
            Ok(Response::builder()
                .status(502)
                .body(full_body(Bytes::from_static(
                    b"502 - Bad Gateway: Lambda Runtime API did not process request",
                )))
                .unwrap())
        }
    }
}

async fn validate_and_mangle_next_event(
    _aws_request_id: Arc<String>,
    response: Response<Incoming>,
) -> Result<Response<ResBody>, Request<ReqBody>> {
    let (parts, body) = response.into_parts();
    let body_bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            tracing::error!(
                "[{}] Failed to read event payload body: {}",
                crate::log_prefix(),
                e
            );
            Bytes::new()
        }
    };

    // Must run before process_payload masks the event below: masking
    // corrupts structural fields like HTTP API v2's `routeKey` (case-
    // insensitive match on the default `.*key.*` rule), which would
    // otherwise leak the mask placeholder into http.route.
    let (api_gateway_request_attributes, api_gateway_span_name) =
        crate::otlp::span_mutations::extract_api_gateway_request_data_from_raw_event(&body_bytes);

    let payload = process_payload(&String::from_utf8_lossy(&body_bytes));

    let event_log = build_payload_log(
        &payload,
        "lambda_event",
        _aws_request_id.as_ref(),
        None,
        None,
        None,
    );
    invocation_entry::update(&_aws_request_id, |entry| {
        entry.event_payload = Some(payload);
        entry.api_gateway_request_attributes = api_gateway_request_attributes;
        entry.api_gateway_span_name = api_gateway_span_name;
        if let Some(log) = event_log {
            entry.logs.push(log);
        }
    });

    // Reconstruct the response with the same parts and body
    let response = Response::from_parts(parts, full_body(body_bytes));

    Ok(response)
}

#[cfg(test)]
mod tests {
    //! Full request -> response round trip through the extension's own
    //! `route::dispatch`, against a mock "sandbox" Lambda Runtime API. This
    //! exercises `proxy_invocation_next` and `invocation_response_proxy` as
    //! real HTTP handlers (not just the pure extraction functions they call),
    //! proving the API Gateway HTTP attributes actually land on the stored
    //! handler span end to end.
    //!
    //! `config::endpoints::sandbox_runtime_api()` is backed by a process-wide
    //! `OnceCell` that latches on first use, so every test in this binary
    //! that drives a real dispatcher must talk to the *same* mock sandbox —
    //! see `shared_sandbox`. This module must be the only place in the test
    //! binary that touches `AWS_LAMBDA_RUNTIME_API` / `sandbox_runtime_api()`,
    //! and its tests are `#[serial]` to avoid racing each other.

    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Incoming;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use prost::Message;
    use serial_test::serial;
    use tokio::net::TcpListener;

    use crate::route;
    use crate::state::invocation_entry;

    fn boxed(bytes: impl Into<Bytes>) -> http_body_util::combinators::BoxBody<Bytes, Infallible> {
        Full::new(bytes.into())
            .map_err(|never: Infallible| match never {})
            .boxed()
    }

    type PendingNext = Arc<Mutex<Option<(String, String)>>>;
    type CapturedResponses = Arc<Mutex<HashMap<String, String>>>;

    struct SharedSandbox {
        addr: String,
        pending_next: PendingNext,
        captured_responses: CapturedResponses,
    }

    static SHARED_SANDBOX: std::sync::OnceLock<SharedSandbox> = std::sync::OnceLock::new();

    /// A minimal stand-in for the real Lambda Runtime API, started once per
    /// test binary and shared by every test: serves one queued event per
    /// `GET .../invocation/next` and records whatever is POSTed to
    /// `.../<id>/response`, keyed by invocation id. Must be shared (not
    /// spawned per test) because `sandbox_runtime_api()` is a process-wide
    /// `OnceCell` — every dispatcher in this binary ends up talking to
    /// whichever sandbox address latched in first.
    ///
    /// Runs on its own dedicated OS thread with its own Tokio runtime,
    /// rather than being `tokio::spawn`ed from within a test. Each
    /// `#[tokio::test]` gets a fresh, short-lived runtime, so a task spawned
    /// from inside test A's runtime is dropped the moment test A returns —
    /// test B would then hang forever waiting on `/next`. A dedicated thread
    /// outlives every individual test's runtime.
    fn shared_sandbox() -> &'static SharedSandbox {
        SHARED_SANDBOX.get_or_init(|| {
            let pending_next: PendingNext = Arc::new(Mutex::new(None));
            let captured_responses: CapturedResponses = Arc::new(Mutex::new(HashMap::new()));
            let (addr_tx, addr_rx) = std::sync::mpsc::channel();

            let pending_next_bg = pending_next.clone();
            let captured_responses_bg = captured_responses.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build sandbox runtime");
                rt.block_on(async move {
                    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let addr = listener.local_addr().unwrap().to_string();
                    addr_tx.send(addr).expect("test thread receiver dropped");

                    loop {
                        let (stream, _) = match listener.accept().await {
                            Ok(pair) => pair,
                            Err(_) => continue,
                        };
                        let io = TokioIo::new(stream);
                        let pending_next = pending_next_bg.clone();
                        let captured_responses = captured_responses_bg.clone();
                        tokio::spawn(async move {
                            let svc = service_fn(move |req: Request<Incoming>| {
                                let pending_next = pending_next.clone();
                                let captured_responses = captured_responses.clone();
                                async move {
                                    let path = req.uri().path().to_string();
                                    let method = req.method().clone();
                                    let response = if method == Method::GET
                                        && path.ends_with("/invocation/next")
                                    {
                                        let queued = pending_next.lock().unwrap().take();
                                        match queued {
                                            Some((event_body, request_id)) => Response::builder()
                                                .header(
                                                    "lambda-runtime-aws-request-id",
                                                    request_id.as_str(),
                                                )
                                                .body(boxed(event_body))
                                                .unwrap(),
                                            None => Response::builder()
                                                .status(500)
                                                .body(boxed(Bytes::from_static(
                                                    b"test bug: no event queued before /next",
                                                )))
                                                .unwrap(),
                                        }
                                    } else if method == Method::POST && path.contains("/response") {
                                        let request_id =
                                            crate::util::parsers::extract_invocation_id_from_path(
                                                &path,
                                            )
                                            .unwrap_or_default();
                                        let body_bytes =
                                            req.into_body().collect().await.unwrap().to_bytes();
                                        captured_responses.lock().unwrap().insert(
                                            request_id,
                                            String::from_utf8_lossy(&body_bytes).to_string(),
                                        );
                                        Response::builder()
                                            .status(202)
                                            .body(boxed(Bytes::new()))
                                            .unwrap()
                                    } else {
                                        Response::builder()
                                            .status(404)
                                            .body(boxed(Bytes::new()))
                                            .unwrap()
                                    };
                                    Ok::<_, Infallible>(response)
                                }
                            });
                            let _ = http1::Builder::new().serve_connection(io, svc).await;
                        });
                    }
                });
            });

            let addr = addr_rx.recv().expect("sandbox thread died before binding");
            SharedSandbox {
                addr,
                pending_next,
                captured_responses,
            }
        })
    }

    /// Queues the event the shared sandbox will serve on the next `/next`
    /// call, and returns the sandbox's address to point the dispatcher at.
    async fn queue_sandbox_event(event_body: String, request_id: String) -> String {
        let sandbox = shared_sandbox();
        *sandbox.pending_next.lock().unwrap() = Some((event_body, request_id));
        sandbox.addr.clone()
    }

    /// Reads (without removing) whatever was POSTed to
    /// `.../<request_id>/response`. Panics if `shared_sandbox` was never
    /// initialized — call after `queue_sandbox_event`.
    fn sandbox_captured_response(request_id: &str) -> Option<String> {
        SHARED_SANDBOX
            .get()
            .expect("shared_sandbox must be initialized before reading captured responses")
            .captured_responses
            .lock()
            .unwrap()
            .get(request_id)
            .cloned()
    }

    /// Binds the extension's real `route::dispatch` on an ephemeral port, the
    /// same way `main.rs` does, so the test drives it exactly as the Lambda
    /// runtime process would.
    async fn spawn_extension_dispatcher() -> String {
        route::init();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => continue,
                };
                let io = TokioIo::new(stream);
                tokio::spawn(async move {
                    let _ = http1::Builder::new()
                        .serve_connection(io, service_fn(route::dispatch))
                        .await;
                });
            }
        });
        addr.to_string()
    }

    fn v1_api_gateway_event() -> String {
        serde_json::json!({
            "httpMethod": "GET",
            "path": "/pets/123",
            "resource": "/pets/{id}",
            "requestContext": {
                "domainName": "abc123.execute-api.us-east-1.amazonaws.com",
                "identity": {"sourceIp": "1.2.3.4"},
                "protocol": "HTTP/1.1"
            }
        })
        .to_string()
    }

    fn v2_api_gateway_event_with_masking_prone_route_key() -> String {
        // A real HTTP API v2 proxy-integration event shape. routeKey is the
        // field the default `.*key.*` (case-insensitive) masking rule used
        // to corrupt before extraction moved to raw-event bytes.
        serde_json::json!({
            "version": "2.0",
            "routeKey": "GET /pets/{id}",
            "rawPath": "/pets/123",
            "rawQueryString": "",
            "requestContext": {
                "domainName": "abc123.execute-api.us-east-1.amazonaws.com",
                "http": {
                    "method": "GET",
                    "path": "/pets/123",
                    "protocol": "HTTP/1.1",
                    "sourceIp": "1.2.3.4"
                },
                "routeKey": "GET /pets/{id}",
                "stage": "$default"
            }
        })
        .to_string()
    }

    fn otlp_trace_body_for(invocation_id: &str) -> Vec<u8> {
        let span = Span {
            trace_id: vec![1u8; 16],
            span_id: vec![2u8; 8],
            attributes: vec![KeyValue {
                key: "faas.invocation_id".to_string(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(invocation_id.to_string())),
                }),
            }],
            ..Default::default()
        };
        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource::default()),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "opentelemetry.instrumentation.aws_lambda".to_string(),
                        ..Default::default()
                    }),
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        request.encode_to_vec()
    }

    fn find_string_attr(attrs: &[KeyValue], key: &str) -> Option<String> {
        attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
            if let Some(AnyValue {
                value: Some(Value::StringValue(s)),
            }) = &kv.value
            {
                Some(s.clone())
            } else {
                None
            }
        })
    }

    fn find_int_attr(attrs: &[KeyValue], key: &str) -> Option<i64> {
        attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
            if let Some(AnyValue {
                value: Some(Value::IntValue(i)),
            }) = &kv.value
            {
                Some(*i)
            } else {
                None
            }
        })
    }

    #[tokio::test]
    #[serial]
    async fn full_api_gateway_round_trip_via_route_dispatch() {
        // route::init() builds an HTTPS client (for OTLP export) that needs a
        // rustls CryptoProvider installed, same as main.rs does at startup.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let invocation_id = "test-round-trip-req-id";
        let event_body = v1_api_gateway_event();

        let sandbox_addr = queue_sandbox_event(event_body.clone(), invocation_id.to_string()).await;

        // Point the extension's sandbox_runtime_api() at the shared mock.
        // Safe across tests: the OnceCell latches to the same address every
        // time, since there is only ever one shared sandbox in this binary.
        std::env::set_var("AWS_LAMBDA_RUNTIME_API", &sandbox_addr);
        std::env::set_var(
            "DASH0_API_GATEWAY_RESPONSE_HEADERS_TO_CAPTURE",
            "content-type",
        );

        let extension_addr = spawn_extension_dispatcher().await;
        let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build_http();

        // 1. Function runtime asks the extension for the next invocation.
        let next_uri = format!(
            "http://{}/2018-06-01/runtime/invocation/next",
            extension_addr
        );
        let next_req = Request::builder()
            .method(Method::GET)
            .uri(&next_uri)
            .body(Full::new(Bytes::new()))
            .unwrap();
        let next_res = client.request(next_req).await.unwrap();
        assert_eq!(next_res.status(), 200);
        let returned_id = next_res
            .headers()
            .get("lambda-runtime-aws-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(returned_id, invocation_id);
        let next_body = next_res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(next_body, Bytes::from(event_body));

        // 2. The runtime SDK in the function process exports its span
        // through the extension, the same way the real OTel SDK would.
        let traces_uri = format!("http://{}/2018-06-01/traces", extension_addr);
        let traces_req = Request::builder()
            .method(Method::POST)
            .uri(&traces_uri)
            .header("content-type", "application/x-protobuf")
            .body(Full::new(Bytes::from(otlp_trace_body_for(invocation_id))))
            .unwrap();
        let traces_res = client.request(traces_req).await.unwrap();
        assert_eq!(traces_res.status(), 200);

        // 3. The handler returns a proxy-integration response.
        let response_uri = format!(
            "http://{}/2018-06-01/runtime/invocation/{}/response",
            extension_addr, invocation_id
        );
        let return_payload =
            serde_json::json!({"statusCode": 200, "headers": {"content-type": "application/json"}, "body": "{}"})
                .to_string();
        let response_req = Request::builder()
            .method(Method::POST)
            .uri(&response_uri)
            .body(Full::new(Bytes::from(return_payload.clone())))
            .unwrap();
        let response_res = client.request(response_req).await.unwrap();
        assert_eq!(response_res.status(), 202);

        // The extension must still forward the response through to the real
        // (mock) sandbox runtime API unmodified.
        assert_eq!(
            sandbox_captured_response(invocation_id).as_deref(),
            Some(return_payload.as_str())
        );

        // 4. The stored handler span now carries the API Gateway HTTP
        // semconv attributes end to end: request attributes merged when the
        // trace arrived, response attributes merged when the return value
        // arrived.
        let entry = invocation_entry::get(invocation_id)
            .expect("invocation entry should exist after the round trip");
        let attrs = &entry.handler_attributes;

        assert_eq!(
            find_string_attr(attrs, "http.request.method"),
            Some("GET".to_string())
        );
        assert_eq!(
            find_string_attr(attrs, "http.route"),
            Some("/pets/:id".to_string())
        );
        assert_eq!(
            find_string_attr(attrs, "client.address"),
            Some("1.2.3.4".to_string())
        );
        assert_eq!(find_int_attr(attrs, "http.response.status_code"), Some(200));
        assert_eq!(
            find_string_attr(attrs, "http.response.header.content-type"),
            Some("application/json".to_string())
        );

        std::env::remove_var("DASH0_API_GATEWAY_RESPONSE_HEADERS_TO_CAPTURE");
        invocation_entry::remove(invocation_id);
    }

    /// Local, no-deploy-needed reproduction of the routeKey masking bug and
    /// its fix: drives a real HTTP API v2 invocation through the extension's
    /// own runtime-API proxy (the exact code path production traffic takes),
    /// and inspects both the stored (masked) event payload and the final
    /// exported span to confirm http.route and the span name are the real,
    /// unmasked values — not the "****" placeholder.
    #[tokio::test]
    #[serial]
    async fn httpapi_v2_round_trip_does_not_leak_masked_route_key() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let invocation_id = "test-httpapi-v2-masking-req-id";
        let event_body = v2_api_gateway_event_with_masking_prone_route_key();

        let sandbox_addr = queue_sandbox_event(event_body.clone(), invocation_id.to_string()).await;

        std::env::set_var("AWS_LAMBDA_RUNTIME_API", &sandbox_addr);
        std::env::set_var("DASH0_ENABLE_API_GATEWAY_SPAN_NAME", "true");

        let extension_addr = spawn_extension_dispatcher().await;
        let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build_http();

        // 1. Function runtime asks the extension for the next invocation.
        let next_uri = format!(
            "http://{}/2018-06-01/runtime/invocation/next",
            extension_addr
        );
        let next_req = Request::builder()
            .method(Method::GET)
            .uri(&next_uri)
            .body(Full::new(Bytes::new()))
            .unwrap();
        let next_res = client.request(next_req).await.unwrap();
        assert_eq!(next_res.status(), 200);

        // 2. Confirm the precondition: the STORED event payload (used for
        // logging) really is masked. This is expected and unrelated to the
        // fix — only span-attribute/name extraction must avoid it.
        let stored_event_payload = invocation_entry::get_event_payload(invocation_id)
            .expect("event payload should be stored after /next");
        assert!(
            stored_event_payload.contains("\"****\""),
            "expected routeKey to be masked in the stored payload, got: {stored_event_payload}"
        );

        // 3. The runtime SDK exports its span through the extension.
        let traces_uri = format!("http://{}/2018-06-01/traces", extension_addr);
        let traces_req = Request::builder()
            .method(Method::POST)
            .uri(&traces_uri)
            .header("content-type", "application/x-protobuf")
            .body(Full::new(Bytes::from(otlp_trace_body_for(invocation_id))))
            .unwrap();
        let traces_res = client.request(traces_req).await.unwrap();
        assert_eq!(traces_res.status(), 200);

        // 4. The handler returns a proxy-integration response.
        let response_uri = format!(
            "http://{}/2018-06-01/runtime/invocation/{}/response",
            extension_addr, invocation_id
        );
        let response_req = Request::builder()
            .method(Method::POST)
            .uri(&response_uri)
            .body(Full::new(Bytes::from(
                serde_json::json!({"statusCode": 200, "body": "{}"}).to_string(),
            )))
            .unwrap();
        let response_res = client.request(response_req).await.unwrap();
        assert_eq!(response_res.status(), 202);

        // 5. The exported span's http.route attribute must be the real
        // route, not the mask placeholder.
        let entry = invocation_entry::get(invocation_id)
            .expect("invocation entry should exist after the round trip");
        assert_eq!(
            find_string_attr(&entry.handler_attributes, "http.route"),
            Some("/pets/:id".to_string()),
            "http.route must be the real unmasked route"
        );
        assert_ne!(
            find_string_attr(&entry.handler_attributes, "http.route"),
            Some("****".to_string())
        );

        // 6. The span NAME (only reachable via the raw stored trace, since
        // handler_attributes doesn't carry it) must also be unmasked.
        let stored_trace = entry
            .traces
            .first()
            .expect("a trace should have been stored");
        let decoded = ExportTraceServiceRequest::decode(stored_trace.body.as_slice())
            .expect("stored trace should decode as OTLP");
        let span_name = &decoded.resource_spans[0].scope_spans[0].spans[0].name;
        assert_eq!(
            span_name, "GET /pets/:id",
            "the derived span name must use the real route, not the mask placeholder"
        );

        std::env::remove_var("DASH0_ENABLE_API_GATEWAY_SPAN_NAME");
        invocation_entry::remove(invocation_id);
    }
}

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
    //! `OnceCell` that latches on first use. This module must be the only
    //! place in the test binary that touches `AWS_LAMBDA_RUNTIME_API` /
    //! `sandbox_runtime_api()`, and its test is `#[serial]` to avoid racing
    //! itself.

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

    /// A minimal stand-in for the real Lambda Runtime API: serves one event
    /// on `GET .../invocation/next` and captures whatever is POSTed to
    /// `.../response`.
    async fn spawn_mock_sandbox(
        event_body: String,
        request_id: String,
    ) -> (String, Arc<Mutex<Option<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured_response = Arc::new(Mutex::new(None));
        let captured_response_clone = captured_response.clone();

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => continue,
                };
                let io = TokioIo::new(stream);
                let event_body = event_body.clone();
                let request_id = request_id.clone();
                let captured_response = captured_response_clone.clone();
                tokio::spawn(async move {
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let event_body = event_body.clone();
                        let request_id = request_id.clone();
                        let captured_response = captured_response.clone();
                        async move {
                            let path = req.uri().path().to_string();
                            let method = req.method().clone();
                            let response = if method == Method::GET
                                && path.ends_with("/invocation/next")
                            {
                                Response::builder()
                                    .header("lambda-runtime-aws-request-id", request_id.as_str())
                                    .body(boxed(event_body))
                                    .unwrap()
                            } else if method == Method::POST && path.contains("/response") {
                                let body_bytes =
                                    req.into_body().collect().await.unwrap().to_bytes();
                                *captured_response.lock().unwrap() =
                                    Some(String::from_utf8_lossy(&body_bytes).to_string());
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

        (addr.to_string(), captured_response)
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

        let (sandbox_addr, captured_response) =
            spawn_mock_sandbox(event_body.clone(), invocation_id.to_string()).await;

        // Point the extension's sandbox_runtime_api() at our mock. Safe: this
        // OnceCell has not been touched by any other test in this binary.
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
            captured_response.lock().unwrap().as_deref(),
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
}

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
use crate::otlp::masking::mask_json_string;
use crate::otlp::span_mutations::{
    apply_return_value_error_to_stored_traces, build_synthetic_trace,
};
use crate::route::{ReqBody, ResBody};
use crate::state::invocation_data::store_current_invocation_id;
use crate::state::invocation_entry;
use crate::util::parsers::extract_invocation_id_from_path;

static HTTP_CLIENT: Lazy<Client<HttpConnector, ReqBody>> =
    Lazy::new(|| Client::builder(TokioExecutor::new()).build_http());

fn empty_body() -> ResBody {
    Full::new(Bytes::new())
}

fn full_body(bytes: Bytes) -> ResBody {
    Full::new(bytes)
}

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

    let mut req = match Request::builder()
        .method("GET")
        .uri(uri)
        .body(req_empty())
    {
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
pub async fn notfound_passthru_proxy(req: Request<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
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
    let start = Instant::now();

    // Extract and print body
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

    let mut endpoint_req: Request<ReqBody> =
        Request::from_parts(parts, req_from_bytes(body_bytes));
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
            collect_response(res).await
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

async fn collect_response(res: Response<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
    let (parts, body) = res.into_parts();
    let bytes = body.collect().await?.to_bytes();
    Ok(Response::from_parts(parts, full_body(bytes)))
}

pub async fn proxy_invocation_next(req: Request<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
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

pub async fn invocation_response_proxy(req: Request<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
    let start = Instant::now();
    let invocation_id = extract_invocation_id_from_path(req.uri().path());
    let (parts, body) = req.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    let return_payload = mask_json_string(&String::from_utf8_lossy(&body_bytes));
    // Reconstruct as a server-side request (Incoming-bodied) for passthru_proxy.
    // We can't synthesize Incoming directly, so route to a helper that accepts bytes.
    let res = passthru_proxy_bytes(parts, body_bytes).await;
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

/// Same as passthru_proxy but the body has already been buffered to bytes.
async fn passthru_proxy_bytes(
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

    let mut endpoint_req: Request<ReqBody> =
        Request::from_parts(parts, req_from_bytes(body_bytes));
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
            collect_response(res).await
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

    let payload = mask_json_string(&String::from_utf8_lossy(&body_bytes));

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

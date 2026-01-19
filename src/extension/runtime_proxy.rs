use std::sync::Arc;
use std::time::{Duration, Instant};

use hyper::{Body, Error, Request, Response, Uri};
use once_cell::sync::Lazy;

use hyper::HeaderMap;

use crate::config::endpoints;
use crate::config::{is_auto_instrumented_disabled, max_event_payload_size};
use crate::otlp::masking::mask_json_string;
use crate::otlp::span_mutations::{
    add_return_payload_to_lambda_server_spans, build_synthetic_trace,
};
use crate::state::invocation_data::{
    store_current_invocation_id, store_event_payload, store_trace,
};
use crate::util::parsers::extract_invocation_id_from_path;

static HTTP_CLIENT: Lazy<hyper::Client<hyper::client::HttpConnector, Body>> =
    Lazy::new(|| hyper::Client::new());

fn process_payload(payload_bytes: &[u8]) -> String {
    let payload_str = String::from_utf8_lossy(payload_bytes);
    let masked_payload = mask_json_string(&payload_str);

    let max_size = max_event_payload_size();
    if masked_payload.len() > max_size {
        tracing::info!(
            "[{}] Truncating payload from {} to {} bytes.",
            crate::log_prefix(),
            masked_payload.len(),
            max_size
        );
        masked_payload[..max_size].to_string()
    } else {
        masked_payload
    }
}

pub async fn next(headers: &HeaderMap, path: &str) -> Result<(Arc<String>, Response<Body>), Error> {
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
        .body(Body::empty())
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
pub async fn notfound_passthru_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    tracing::info!(
        "[{}] Route not found: path={} method={}",
        crate::log_prefix(),
        &req.uri().path(),
        &req.method()
    );
    passthru_proxy(req).await
}

#[allow(dead_code)]
pub async fn passthru_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();

    // Extract and print body
    let (parts, body) = req.into_parts();
    let body_bytes = match hyper::body::to_bytes(body).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(
                "[{}] Failed to read request body in passthru_proxy: {}",
                crate::log_prefix(),
                e
            );
            return Ok(Response::builder()
                .status(500)
                .body("500 - Internal Error: Failed to read request body".into())
                .unwrap_or_else(|_| Response::new(Body::empty())));
        }
    };

    // Reconstruct request
    let req = Request::from_parts(parts, Body::from(body_bytes));

    let endpoint_client = &*HTTP_CLIENT;
    let endpoint_uri: Uri = match Uri::builder()
        .scheme("http")
        .authority(endpoints::sandbox_runtime_api())
        .path_and_query(req.uri().path())
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
                .body("502 - Bad Gateway: Invalid runtime API configuration".into())
                .unwrap_or_else(|_| Response::new(Body::empty())));
        }
    };

    // remap URI
    let mut endpoint_req: Request<Body> = req.into();
    *endpoint_req.uri_mut() = endpoint_uri.clone();

    let method = endpoint_req.method().clone();

    match endpoint_client.request(endpoint_req).await {
        Ok(res) => {
            tracing::info!(
                "[{}] passthru_proxy - {} {} completed in {} ms",
                crate::log_prefix(),
                method,
                endpoint_uri,
                start.elapsed().as_millis()
            );
            Ok(res)
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
                .body("502 - Bad Gateway: Lambda Runtime API did not process request".into())
                .unwrap())
        }
    }
}

pub async fn proxy_invocation_next(req: Request<Body>) -> Result<Response<Body>, Error> {
    'getNext: loop {
        // track either initialization  -or-
        // how long it took to process the event and request next
        //
        crate::stats::get_next_event();

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
                HTTP_CLIENT.request(req).await.ok();
                continue 'getNext;
            }
        }
    }
}

pub async fn invocation_response_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();
    let invocation_id = extract_invocation_id_from_path(req.uri().path());
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;

    let return_payload = process_payload(&body_bytes);
    let req = Request::from_parts(parts, Body::from(body_bytes));

    let res = passthru_proxy(req).await;
    if let Some(id) = invocation_id {
        if is_auto_instrumented_disabled() {
            if let Some(trace) =
                build_synthetic_trace(&id, None, Some(return_payload.as_str()), &Vec::new())
            {
                store_trace(trace);
            }
        } else {
            if !add_return_payload_to_lambda_server_spans(&id, &return_payload) {
                tracing::info!(
                    "[{}] invocation_response_proxy - no lambda server span found for return value {}", crate::log_prefix(),
                    &id
                );
            }
        }
    }
    tracing::info!(
        "[{}] Total handle time for invocation response: {} ms",
        crate::log_prefix(),
        start.elapsed().as_millis()
    );
    res
}

async fn validate_and_mangle_next_event(
    _aws_request_id: Arc<String>,
    response: Response<Body>,
) -> Result<Response<Body>, Request<Body>> {
    let (parts, body) = response.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap_or_else(|e| {
        tracing::error!(
            "[{}] Failed to read event payload body: {}",
            crate::log_prefix(),
            e
        );
        hyper::body::Bytes::new()
    });

    let processed_payload = process_payload(&body_bytes);
    store_event_payload(&_aws_request_id, &processed_payload);

    // Reconstruct the response with the same parts and body
    let response = Response::from_parts(parts, Body::from(body_bytes));

    Ok(response)
}

//
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: MIT-0
//

//! Routing for Runtime API requests.  This builds out the services and stitches them together as
//! well as builds routing tables for HTTP methods on resources to proxy the Lambda Runtime API.
//!
//!

use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use httprouter::Router;
use hyper::{Body, Error, Request, Response, Uri};
use hyper_rustls::HttpsConnectorBuilder;
use once_cell::sync::Lazy;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::backend_send::flush_logs;
use crate::config::{is_auto_instrumented_disabled, is_send_on_invocation_end};
use crate::store::take_return_payload;
use crate::{
    backend_send::{flush_traces, send_traces},
    env, sandbox, stats,
    store::{
        force_init_trace_store, store_current_invocation_id, store_event_payload,
        store_invocation_start, store_trace, take_event_payload, take_invocation_start,
        take_traces, StoredTrace,
    },
    util::{
        parsers::{extract_error_invocation_ids, extract_invocation_id_from_path},
        span_mutations::{
            add_return_payload_to_lambda_server_spans, build_runtime_error_trace,
            process_trace_request,
        },
        LimitedBuffer,
    },
};

pub fn make_route<'a>() -> Router<'a> {
    // Route `invocation/next` demonstrates hooks for filtering incoming request events
    // Users can implement a similar patern in `invocation/:id/response` to filter responses
    let router = Router::default()
        .get("/", passthru_proxy)
        .get("/:apiver/runtime/invocation/next", proxy_invocation_next)
        .post(
            "/:apiver/runtime/invocation/:id/response",
            invocation_response_proxy,
        )
        .post(
            "/:apiver/runtime/invocation/:id/error",
            invocation_response_proxy,
        )
        .post("/:apiver/traces", traces)
        .post("/:apiver/logs", logs)
        .post("/:apiver/telemetry", telemetry_sink)
        .not_found(notfound_passthru_proxy);
    Lazy::force(&HTTPS_CLIENT);
    force_init_trace_store();
    router
}

/// Pass-through the request, but log the unhandled path and method
#[allow(dead_code)]
pub async fn notfound_passthru_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    tracing::error!(
        "[LRAP] Route not found: path={} method={}",
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
    let body_bytes = hyper::body::to_bytes(body).await.unwrap();

    // Reconstruct request
    let req = Request::from_parts(parts, Body::from(body_bytes));

    // possible improvement: replace with resource pool or persistent connection
    let endpoint_client = hyper::Client::new();
    let endpoint_uri: Uri = Uri::builder()
        .scheme("http")
        .authority(env::sandbox_runtime_api())
        .path_and_query(req.uri().path())
        .build()
        .unwrap();

    // remap URI
    let mut endpoint_req: Request<Body> = req.into();
    *endpoint_req.uri_mut() = endpoint_uri.clone();

    let method = endpoint_req.method().clone();

    match endpoint_client.request(endpoint_req).await {
        Ok(res) => {
            tracing::info!(
                "[LRAP] passthru_proxy - {} {} completed in {} ms",
                method,
                endpoint_uri,
                start.elapsed().as_millis()
            );
            Ok(res)
        }
        Err(e) => {
            tracing::error!(
                "[LRAP] Error invoking endpoint ({} on {}): {:?}",
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

#[allow(dead_code)]
pub async fn logs(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();

    // Extract and print body
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap();

    // Reconstruct request
    let req = Request::from_parts(parts, Body::from(body_bytes));

    // use TLS-enabled client for the https target
    let endpoint_client = HTTPS_CLIENT.clone();
    let target_authority = "e7ombfy3t62jczfmcfrwdgzlyu0anjhb.lambda-url.us-west-2.on.aws:443";
    let endpoint_uri: Uri = Uri::builder()
        .scheme("https")
        .authority(target_authority)
        .path_and_query(
            req.uri()
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/"),
        )
        .build()
        .unwrap();

    // remap URI
    let mut endpoint_req: Request<Body> = req.into();
    *endpoint_req.uri_mut() = endpoint_uri.clone();
    // ensure Host header matches target authority (not the inbound :9009)
    if let Ok(host_value) = hyper::header::HeaderValue::from_str(target_authority) {
        endpoint_req
            .headers_mut()
            .insert(hyper::header::HOST, host_value);
    }

    let method = endpoint_req.method().clone();

    match endpoint_client.request(endpoint_req).await {
        Ok(res) => {
            tracing::info!(
                "[LRAP] v1/logs - {} {} completed in {} ms. status code={}",
                method,
                endpoint_uri,
                start.elapsed().as_millis(),
                res.status()
            );
            Ok(res)
        }
        Err(e) => {
            tracing::error!(
                "[LRAP] Error invoking endpoint ({} on {}): {:?}",
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

pub async fn telemetry_sink(req: Request<Body>) -> Result<Response<Body>, Error> {
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body_bytes);

    let error_invocation_ids = extract_error_invocation_ids(&body_bytes, body_text.as_ref());

    tracing::info!(
        "[LRAP] telemetry event path={} len={} body={}",
        parts.uri.path(),
        body_bytes.len(),
        body_text
    );

    if let Ok(mut logs) = serde_json::from_str::<Vec<crate::store::TelemetryLog>>(&body_text) {
        let mut current_invocation_id = crate::store::get_last_seen_invocation_start();

        for log in &mut logs {
            if log.r#type == "platform.start" {
                if let Some(record) = log.record.as_object() {
                    if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
                        crate::store::store_last_seen_invocation_start(req_id);
                        current_invocation_id = Some(req_id.to_string());
                    }
                }
            }

            // Signal when platform.runtimeDone is received
            if log.r#type == "platform.runtimeDone" {
                if let Some(notifier) = crate::store::take_runtime_done_notifier() {
                    // Signal the waiting task
                    tracing::info!("[LRAP] Signaled platform.runtimeDone");
                    let _ = notifier.send(());
                }
            }

            // For platform logs, extract invocation ID from the log record itself (safer than state)
            // For other logs, use the current invocation ID from state
            let invocation_id = if log.r#type == "platform.start"
                || log.r#type == "platform.runtimeDone"
                || log.r#type == "platform.report"
            {
                log.record
                    .as_object()
                    .and_then(|record| record.get("requestId"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| current_invocation_id.clone())
            } else {
                current_invocation_id.clone()
            };

            log.invocation_id = invocation_id;
        }
        crate::store::store_telemetry_logs(logs);
    } else {
        tracing::debug!("[LRAP] Failed to deserialize telemetry logs from body");
    }

    let env_vars: Vec<String> = std::env::vars()
        .map(|(key, value)| format!("{}={}", key, value))
        .collect();
    tracing::trace!("[LRAP] telemetry environment: {}", env_vars.join(", "));

    if !error_invocation_ids.is_empty() {
        tracing::info!(
            "[LRAP] telemetry runtimeDone error detected for invocations: {:?} body={}",
            error_invocation_ids,
            body_text
        );

        // Fetch account ID if not already cached
        if sandbox::get_account_id()
            .map(|id| id.is_empty())
            .unwrap_or(true)
        {
            let _ = tokio::task::spawn(async { sandbox::fetch_and_cache_account_id().await }).await;
        }

        let mut traces_to_send = take_traces();

        for (invocation_id, error_type) in &error_invocation_ids {
            match build_runtime_error_trace(invocation_id, Some(error_type), None, &traces_to_send)
            {
                Some(trace) => traces_to_send.push(trace),
                None => {
                    tracing::error!(
                        "[LRAP] Failed to build runtimeDone trace for invocation {}",
                        invocation_id
                    );
                }
            }
        }

        if !traces_to_send.is_empty() {
            send_traces(traces_to_send).await;
        }
        flush_logs().await;
    }

    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

pub async fn invocation_response_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();
    let invocation_id = extract_invocation_id_from_path(req.uri().path());
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;
    let return_payload = String::from_utf8_lossy(&body_bytes).to_string();
    let req = Request::from_parts(parts, Body::from(body_bytes));

    let res = passthru_proxy(req).await;
    if let Some(id) = invocation_id {
        if is_auto_instrumented_disabled() {
            if let Some(trace) =
                build_runtime_error_trace(&id, None, Some(return_payload.as_str()), &Vec::new())
            {
                store_trace(trace);
            }
        } else {
            if !add_return_payload_to_lambda_server_spans(&id, &return_payload) {
                tracing::info!(
                    "[LRAP] invocation_response_proxy - no lambda server span found for return value {}",
                    &id
                );
            }
        }
    }
    if is_send_on_invocation_end() {
        flush_traces().await;
        flush_logs().await;
    }
    tracing::info!(
        "[LRAP] Total handle time for invocation response: {} ms",
        start.elapsed().as_millis()
    );
    res
}

pub async fn traces(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;

    // Try to decode and add event payload to server span from AWS Lambda instrumentation
    let mut encoded_body: Vec<u8> = body_bytes.to_vec();
    let mut invocation_ids: Vec<String> = Vec::new();
    match ExportTraceServiceRequest::decode(body_bytes.as_ref()) {
        Ok(mut decoded) => {
            process_trace_request(&mut decoded, &mut invocation_ids, &mut encoded_body);
        }
        Err(err) => tracing::error!("[LRAP] /v1/traces failed to decode OTLP: {}", err),
    }

    if invocation_ids.is_empty() {
        if let Some(current) = crate::store::get_current_invocation_id() {
            invocation_ids.push(current);
        }
    }

    store_trace(StoredTrace {
        method: parts.method,
        path_and_query: parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string()),
        headers: parts.headers,
        body: encoded_body,
        invocation_ids,
    });

    tracing::info!(
        "[LRAP] Total handle time for /v1/traces {} ms",
        start.elapsed().as_millis()
    );
    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

pub(crate) fn cleanup_invocation(invocation_id: &str) {
    take_event_payload(invocation_id);
    take_invocation_start(invocation_id);
    take_return_payload(invocation_id);
}

pub(crate) static HTTPS_CLIENT: Lazy<
    hyper::Client<hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>, Body>,
> = Lazy::new(|| {
    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_only()
        .enable_http1()
        .build();
    hyper::Client::builder().build::<_, Body>(https)
});

/// Example of reading the HTTP body into a limited-length buffer for later processing
#[allow(dead_code)]
async fn hyper_body_to_body_buffer(
    size: usize,
    body: hyper::Body,
) -> std::sync::Arc<LimitedBuffer> {
    use futures::stream::StreamExt;
    use tokio_util::io::StreamReader;

    let mut body_buffer = LimitedBuffer::new(size);

    let mapped_stream = body.map(|chunk_result| {
        chunk_result.map_err(|hyper_err| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Hyper error: {}", hyper_err),
            )
        })
    });

    let mut reader = StreamReader::new(mapped_stream);
    tokio::io::copy(&mut reader, &mut body_buffer)
        .await
        .unwrap();
    std::sync::Arc::new(body_buffer)
}

/// Get next invocation; provide hooks for skipping bad requests (payload malicious or ill-formed)
///
/// Flow:
///
///          [App Runtime]               [LRAP]                        [Lambda Service]
///               |                         
///               +---- GET next event --->|
///                                        |
///                                 [ proxy request ]-- GET next event ------>|
///                                                                           |                             
///                                                                           |<---- [ INVOKE with payload ]
///                                        |<--------- event payload ---------|
///                                        |                                   
///                          [ if validation fails: DROP event ]                  
///                                        |                                   
///                                        |----------- GET next event ------>|
///                                                                           |<---- [ INVOKE with payload ]
///                                        |<--------- event payload ---------|
///                                        |                                   
///               |<-- event -----[ if validation succeeds: PASS event ]               
///               |   payload             
///               |                         
///           [ appp logic ]                
///               |                         
///               |--response payload ---->|
///                                        |                                   
///                              [ sanitize response ]-- response sanitized ->|
///                                                                           |----->[ synchronous response ]
///                                         
pub async fn proxy_invocation_next(req: Request<Body>) -> Result<Response<Body>, Error> {
    use std::time::Duration;

    'getNext: loop {
        // track either initialization  -or-
        // how long it took to process the event and request next
        //
        stats::get_next_event();

        let (aws_request_id, response) =
            match crate::sandbox::next(req.headers(), req.uri().path()).await {
                Err(e) => {
                    tracing::error!(
                        "[LRAP]  Error getting next invocation from Runtime API: {}",
                        e
                    );
                    tracing::trace!("[LRAP] uri: {}", req.uri());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue 'getNext;
                }
                Ok(response) => response,
            };

        // start the counter on the new event
        stats::event_start();

        store_current_invocation_id(aws_request_id.as_str());

        if let Ok(nanos) = SystemTime::now().duration_since(UNIX_EPOCH) {
            store_invocation_start(aws_request_id.as_str(), nanos.as_nanos() as u64);
        }

        tracing::info!("[LRAP] Got invocation next: {}", aws_request_id.as_str());

        match validate_and_mangle_next_event(aws_request_id, response).await {
            Ok(response) => {
                return Ok(response);
            }
            Err(req) => {
                sandbox::send_request(req).await.ok();
                continue 'getNext;
            }
        }
    }
}

/// Process the next invocation event from the Lambda Runtime API
///
/// Event context, payload is in `response`
///
/// On Error, create a [`Request<Body>`] to send to the Runtime API.
///
/// This _could_ be a request to the Runtime API's /runtime/invocation/:id/response to short-cut the Application with a specific code
///
async fn validate_and_mangle_next_event(
    _aws_request_id: Arc<String>,
    response: Response<Body>,
) -> Result<Response<Body>, Request<Body>> {
    let (parts, body) = response.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap();

    tracing::trace!("[LRAP] aws request id: {}", _aws_request_id);
    tracing::trace!(
        "[LRAP] event payload: {}",
        String::from_utf8_lossy(&body_bytes)
    );
    store_event_payload(&_aws_request_id, &String::from_utf8_lossy(&body_bytes));

    // Reconstruct the response with the same parts and body
    let response = Response::from_parts(parts, Body::from(body_bytes));

    return Ok(response);
}

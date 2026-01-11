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

use crate::backend_send::{flush_logs, flush_traces};
use crate::config::{is_auto_instrumented_disabled, max_event_payload_size};
use crate::{
    backend_send::send_traces,
    env, sandbox, stats,
    store::{
        force_init_trace_store, store_current_invocation_id, store_event_payload,
        store_invocation_end, store_invocation_start, store_trace, take_traces, StoredTrace,
    },
    util::{
        parsers::{extract_error_invocation_ids, extract_invocation_id_from_path},
        span_mutations::{
            add_return_payload_to_lambda_server_spans, build_runtime_error_trace,
            drop_duplicate_java_instrumenations, process_trace_request,
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
        "[{}] Route not found: path={} method={}", crate::log_prefix(),
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
                "[{}] Failed to read request body in passthru_proxy: {}", crate::log_prefix(),
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

    // possible improvement: replace with resource pool or persistent connection
    let endpoint_client = hyper::Client::new();
    let endpoint_uri: Uri = match Uri::builder()
        .scheme("http")
        .authority(env::sandbox_runtime_api())
        .path_and_query(req.uri().path())
        .build()
    {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!("[{}] Failed to build URI for sandbox runtime API: {}", crate::log_prefix(), e);
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
                "[{}] passthru_proxy - {} {} completed in {} ms", crate::log_prefix(),
                method,
                endpoint_uri,
                start.elapsed().as_millis()
            );
            Ok(res)
        }
        Err(e) => {
            tracing::error!(
                "[{}] Error invoking endpoint ({} on {}): {:?}", crate::log_prefix(),
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
        "[{}] telemetry event path={} len={} body={}", crate::log_prefix(),
        parts.uri.path(),
        body_bytes.len(),
        body_text
    );

    if let Ok(mut logs) = serde_json::from_str::<Vec<crate::store::TelemetryLog>>(&body_text) {
        crate::util::log_processing::process_telemetry_logs(&mut logs);

        let mut report_invocation_ids: Vec<String> = Vec::new();
        for log in &logs {
            if log.r#type == "platform.report" {
                if let Some(id) = &log.invocation_id {
                    report_invocation_ids.push(id.clone());
                }
            }
        }
        crate::store::store_telemetry_logs(logs);

        if !report_invocation_ids.is_empty() {
            flush_traces().await;
        }
    } else {
        tracing::debug!("[{}] Failed to deserialize telemetry logs from body", crate::log_prefix());
    }

    let env_vars: Vec<String> = std::env::vars()
        .map(|(key, value)| format!("{}={}", key, value))
        .collect();
    tracing::trace!("[{}] telemetry environment: {}", crate::log_prefix(), env_vars.join(", "));

    if !error_invocation_ids.is_empty() {
        tracing::info!(
            "[{}] telemetry runtimeDone error detected for invocations: {:?} body={}", crate::log_prefix(),
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
                        "[{}] Failed to build runtimeDone trace for invocation {}", crate::log_prefix(),
                        invocation_id
                    );
                }
            }
        }

        if !traces_to_send.is_empty() {
            send_traces(traces_to_send).await;
        }
        flush_logs(true).await;
    }

    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

pub async fn invocation_response_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();
    let invocation_id = extract_invocation_id_from_path(req.uri().path());
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;

    let max_size = max_event_payload_size();
    let payload_slice = if body_bytes.len() > max_size {
        tracing::info!(
            "[{}] Truncating return payload from {} to {} bytes", crate::log_prefix(),
            body_bytes.len(),
            max_size
        );
        &body_bytes[..max_size]
    } else {
        &body_bytes
    };
    let return_payload = String::from_utf8_lossy(payload_slice).to_string();
    let req = Request::from_parts(parts, Body::from(body_bytes));

    let res = passthru_proxy(req).await;
    if let Some(id) = invocation_id {
        if let Ok(nanos) = SystemTime::now().duration_since(UNIX_EPOCH) {
            store_invocation_end(&id, nanos.as_nanos() as u64);
        }

        if is_auto_instrumented_disabled() {
            if let Some(trace) =
                build_runtime_error_trace(&id, None, Some(return_payload.as_str()), &Vec::new())
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
        "[{}] Total handle time for invocation response: {} ms", crate::log_prefix(),
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
    let mut converted_from_json = false;

    tracing::trace!(
        "[{}] /v1/traces body: {}", crate::log_prefix(),
        String::from_utf8_lossy(&encoded_body)
    );

    match ExportTraceServiceRequest::decode(body_bytes.as_ref()) {
        Ok(mut decoded) => {
            if drop_duplicate_java_instrumenations(&decoded) {
                return Ok(Response::builder().status(200).body(Body::empty()).unwrap());
            }

            process_trace_request(&mut decoded, &mut invocation_ids, &mut encoded_body);
        }
        Err(err) => {
            tracing::info!(
                "[{}] /v1/traces failed to decode as protobuf, trying JSON: {}", crate::log_prefix(),
                err
            );

            // Try to parse as JSON and convert to protobuf
            match serde_json::from_slice::<ExportTraceServiceRequest>(body_bytes.as_ref()) {
                Ok(mut decoded) => {
                    for resource_span in &mut decoded.resource_spans {
                        for scope_span in &mut resource_span.scope_spans {
                            for span in &mut scope_span.spans {
                                for attribute in &mut span.attributes {
                                    if attribute.key == "faas.execution" {
                                        attribute.key = "faas.invocation_id".to_string();
                                    } else if attribute.key == "faas.id" {
                                        attribute.key = "cloud.resource_id".to_string();
                                    }
                                }
                            }
                        }
                    }

                    // Convert to protobuf format for storage
                    // This ensures encoded_body contains protobuf bytes before calling process_trace_request
                    encoded_body = decoded.encode_to_vec();

                    process_trace_request(&mut decoded, &mut invocation_ids, &mut encoded_body);
                    converted_from_json = true;
                }
                Err(json_err) => {
                    tracing::error!("[{}] /v1/traces failed to parse as JSON: {}", crate::log_prefix(), json_err);
                }
            }
        }
    }

    if invocation_ids.is_empty() {
        if let Some(current) = crate::store::get_current_invocation_id() {
            invocation_ids.push(current);
        }
    }

    // If we converted from JSON to protobuf, update the Content-Type header
    let mut headers = parts.headers;
    if converted_from_json {
        headers.insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/x-protobuf"),
        );
    }

    let seen_invocation_ids = invocation_ids.clone();
    store_trace(StoredTrace {
        method: parts.method,
        path_and_query: parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string()),
        headers,
        body: encoded_body,
        invocation_ids,
    });

    tracing::info!(
        "[{}] Total handle time for /v1/traces {} ms. seen invocation ids: {:?}", crate::log_prefix(),
        start.elapsed().as_millis(),
        seen_invocation_ids
    );
    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
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
                        "[{}]  Error getting next invocation from Runtime API: {}", crate::log_prefix(),
                        e
                    );
                    tracing::trace!("[{}] uri: {}", crate::log_prefix(), req.uri());
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

        tracing::info!("[{}] Got invocation next: {}", crate::log_prefix(), aws_request_id.as_str());

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
    let body_bytes = match hyper::body::to_bytes(body).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("[{}] Failed to read event payload body: {}", crate::log_prefix(), e);
            // Return empty body rather than crashing
            hyper::body::Bytes::new()
        }
    };

    tracing::trace!("[{}] aws request id: {}", crate::log_prefix(), _aws_request_id);
    tracing::trace!(
        "[{}] event payload: {}", crate::log_prefix(),
        String::from_utf8_lossy(&body_bytes)
    );

    let max_size = max_event_payload_size();
    let truncated_bytes = if body_bytes.len() > max_size {
        tracing::info!(
            "[{}] Truncating event payload from {} to {} bytes.", crate::log_prefix(),
            body_bytes.len(),
            max_size
        );
        &body_bytes[..max_size]
    } else {
        &body_bytes
    };
    store_event_payload(&_aws_request_id, &String::from_utf8_lossy(truncated_bytes));

    // Reconstruct the response with the same parts and body
    let response = Response::from_parts(parts, Body::from(body_bytes));

    return Ok(response);
}

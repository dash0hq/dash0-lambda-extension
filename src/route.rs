use std::time::Instant;

use httprouter::Router;
use hyper::{Body, Error, Request, Response};
use hyper_rustls::HttpsConnectorBuilder;
use once_cell::sync::Lazy;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::backend_send::{flush_logs, flush_traces, send_traces};
use crate::state;
use crate::store::{force_init_trace_store, store_trace, take_traces, StoredTrace};
use crate::util::parsers::extract_error_invocation_ids;
use crate::util::span_mutations::{
    build_runtime_error_trace, drop_duplicate_java_instrumenations, process_trace_request,
};

pub fn make_route<'a>() -> Router<'a> {
    let router = Router::default()
        .get("/", crate::extension::runtime_proxy::passthru_proxy)
        .get(
            "/:apiver/runtime/invocation/next",
            crate::extension::runtime_proxy::proxy_invocation_next,
        )
        .post(
            "/:apiver/runtime/invocation/:id/response",
            crate::extension::runtime_proxy::invocation_response_proxy,
        )
        .post(
            "/:apiver/runtime/invocation/:id/error",
            crate::extension::runtime_proxy::invocation_response_proxy,
        )
        .post("/:apiver/traces", traces)
        .post("/:apiver/telemetry", telemetry_sink)
        .not_found(crate::extension::runtime_proxy::notfound_passthru_proxy);
    Lazy::force(&HTTPS_CLIENT);
    force_init_trace_store();
    router
}

pub async fn telemetry_sink(req: Request<Body>) -> Result<Response<Body>, Error> {
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body_bytes);

    let error_invocation_ids = extract_error_invocation_ids(&body_bytes, body_text.as_ref());

    tracing::info!(
        "[{}] telemetry event path={} len={} body={}",
        crate::log_prefix(),
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
            for id in &report_invocation_ids {
                crate::store::cleanup_invocation(id);
            }
        }
    } else {
        tracing::debug!(
            "[{}] Failed to deserialize telemetry logs from body",
            crate::log_prefix()
        );
    }

    if !error_invocation_ids.is_empty() {
        tracing::info!(
            "[{}] telemetry runtimeDone error detected for invocations: {:?} body={}",
            crate::log_prefix(),
            error_invocation_ids,
            body_text
        );

        // Fetch account ID if not already cached
        if state::global::get_account_id()
            .map(|id| id.is_empty())
            .unwrap_or(true)
        {
            let _ = tokio::task::spawn(async { state::global::fetch_and_cache_account_id().await })
                .await;
        }

        let mut traces_to_send = take_traces();

        for (invocation_id, error_type) in &error_invocation_ids {
            match build_runtime_error_trace(invocation_id, Some(error_type), None, &traces_to_send)
            {
                Some(trace) => traces_to_send.push(trace),
                None => {
                    tracing::error!(
                        "[{}] Failed to build runtimeDone trace for invocation {}",
                        crate::log_prefix(),
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

pub async fn traces(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;

    // Try to decode and add event payload to server span from AWS Lambda instrumentation
    let mut encoded_body: Vec<u8> = body_bytes.to_vec();
    let mut invocation_ids: Vec<String> = Vec::new();
    let mut converted_from_json = false;

    tracing::trace!(
        "[{}] /v1/traces body: {}",
        crate::log_prefix(),
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
                "[{}] /v1/traces failed to decode as protobuf, trying JSON: {}",
                crate::log_prefix(),
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
                    tracing::error!(
                        "[{}] /v1/traces failed to parse as JSON: {}",
                        crate::log_prefix(),
                        json_err
                    );
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
        "[{}] Total handle time for /v1/traces {} ms. seen invocation ids: {:?}",
        crate::log_prefix(),
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

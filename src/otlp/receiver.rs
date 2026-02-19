use std::time::Instant;

use hyper::{Body, Error, Request, Response};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::otlp::span_mutations::{drop_duplicate_java_instrumenations, process_trace_request};
use crate::state::invocation_data::StoredTrace;
use crate::state::invocation_entry;

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
        if let Some(current) = crate::state::invocation_data::get_current_invocation_id() {
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
    invocation_entry::store_trace(StoredTrace {
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

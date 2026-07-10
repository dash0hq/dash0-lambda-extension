use std::time::Instant;

use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use prost::Message;

use crate::route::{empty_body, ResBody};
use crate::state::invocation_data::{store_log, StoredLog};

pub async fn logs(req: Request<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
    let start = Instant::now();
    let (parts, body) = req.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    let mut encoded_body: Vec<u8> = body_bytes.to_vec();
    let mut converted_from_json = false;

    tracing::info!(
        "[{}] /v1/logs body: {}",
        crate::log_prefix(),
        String::from_utf8_lossy(&encoded_body)
    );

    match ExportLogsServiceRequest::decode(body_bytes.as_ref()) {
        Ok(decoded) => {
            tracing::debug!(
                "[{}] /v1/logs decoded {} resource_logs",
                crate::log_prefix(),
                decoded.resource_logs.len()
            );
        }
        Err(err) => {
            tracing::info!(
                "[{}] /v1/logs failed to decode as protobuf, trying JSON: {}",
                crate::log_prefix(),
                err
            );

            match serde_json::from_slice::<ExportLogsServiceRequest>(body_bytes.as_ref()) {
                Ok(decoded) => {
                    encoded_body = decoded.encode_to_vec();
                    converted_from_json = true;
                    tracing::debug!(
                        "[{}] /v1/logs decoded from JSON {} resource_logs",
                        crate::log_prefix(),
                        decoded.resource_logs.len()
                    );
                }
                Err(json_err) => {
                    tracing::error!(
                        "[{}] /v1/logs failed to parse as JSON: {}",
                        crate::log_prefix(),
                        json_err
                    );
                }
            }
        }
    }

    let mut headers = parts.headers;
    if converted_from_json {
        headers.insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/x-protobuf"),
        );
    }

    store_log(StoredLog {
        method: parts.method,
        path_and_query: parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string()),
        headers,
        body: encoded_body,
    });

    tracing::info!(
        "[{}] Total handle time for /v1/logs {} ms",
        crate::log_prefix(),
        start.elapsed().as_millis(),
    );
    Ok(Response::builder().status(200).body(empty_body()).unwrap())
}

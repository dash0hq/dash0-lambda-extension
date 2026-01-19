use std::time::Instant;

use hyper::{Body, Error, Request, Response};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::otlp::span_mutations::{drop_duplicate_java_instrumenations, process_trace_request};
use crate::state::invocation_data::{store_trace, StoredTrace};

pub async fn logs(req: Request<Body>) -> Result<Response<Body>, Error> {
    let start = Instant::now();
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;

    // Try to decode and add event payload to server span from AWS Lambda instrumentation
    let mut encoded_body: Vec<u8> = body_bytes.to_vec();

    tracing::trace!(
        "[{}] /v1/traces body: {}",
        crate::log_prefix(),
        String::from_utf8_lossy(&encoded_body)
    );


    tracing::info!(
        "[{}] Total handle time for /v1/logs {} ms. ",
        crate::log_prefix(),
        start.elapsed().as_millis(),
    );
    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

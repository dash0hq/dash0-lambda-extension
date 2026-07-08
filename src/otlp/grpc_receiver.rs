//! OTLP/gRPC receiver.
//!
//! Accepts telemetry from the runtime on the default OpenTelemetry gRPC port
//! (4317) and feeds it into the same processing pipeline as the OTLP/HTTP
//! receivers. Payloads are re-encoded as protobuf and stored as if they had
//! been received on the corresponding OTLP/HTTP path, since the exporter
//! forwards them to the backend via OTLP/HTTP.

use std::net::SocketAddr;

use bytes::Bytes;
use hyper::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use hyper::Method;
use opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::{
    LogsService, LogsServiceServer,
};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::collector::metrics::v1::metrics_service_server::{
    MetricsService, MetricsServiceServer,
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message;
use tonic::{Request, Response, Status};

use crate::otlp::logs_receiver::handle_logs_payload;
use crate::otlp::metrics_receiver::handle_metrics_payload;
use crate::otlp::receiver::handle_traces_payload;

/// Headers for a stored payload received via gRPC: the body is always
/// protobuf-encoded, matching what the OTLP/HTTP exporter sends upstream.
fn protobuf_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-protobuf"),
    );
    headers
}

#[derive(Default)]
pub struct OtlpGrpcTraceService;

#[tonic::async_trait]
impl TraceService for OtlpGrpcTraceService {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let body = request.into_inner().encode_to_vec();
        handle_traces_payload(
            Method::POST,
            "/v1/traces".to_string(),
            protobuf_headers(),
            Bytes::from(body),
        );
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

#[derive(Default)]
pub struct OtlpGrpcLogsService;

#[tonic::async_trait]
impl LogsService for OtlpGrpcLogsService {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let body = request.into_inner().encode_to_vec();
        handle_logs_payload(
            Method::POST,
            "/v1/logs".to_string(),
            protobuf_headers(),
            Bytes::from(body),
        );
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}

#[derive(Default)]
pub struct OtlpGrpcMetricsService;

#[tonic::async_trait]
impl MetricsService for OtlpGrpcMetricsService {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let body = request.into_inner().encode_to_vec();
        handle_metrics_payload(
            Method::POST,
            "/v1/metrics".to_string(),
            protobuf_headers(),
            Bytes::from(body),
        );
        Ok(Response::new(ExportMetricsServiceResponse::default()))
    }
}

/// Serves the OTLP/gRPC receiver on the given address. Only returns on
/// server failure.
pub async fn serve(addr: SocketAddr) {
    tracing::info!(
        "[{}] OTLP/gRPC receiver listening on {}",
        crate::log_prefix(),
        addr
    );

    if let Err(e) = tonic::transport::Server::builder()
        .add_service(TraceServiceServer::new(OtlpGrpcTraceService))
        .add_service(LogsServiceServer::new(OtlpGrpcLogsService))
        .add_service(MetricsServiceServer::new(OtlpGrpcMetricsService))
        .serve(addr)
        .await
    {
        tracing::error!("[{}] OTLP/gRPC receiver failed: {}", crate::log_prefix(), e);
        panic!(
            "[{}] Cannot continue without OTLP/gRPC receiver",
            crate::log_prefix()
        );
    }
}

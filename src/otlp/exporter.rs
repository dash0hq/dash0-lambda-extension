use std::time::Duration;

use hyper::{header, Body, Request, Uri};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::config::{request_retries, request_timeout_ms};
use crate::otlp::log_mutations::{get_resources_attributes, map_logs_to_otlp};
use crate::otlp::span_mutations::merge_telemetry_invocation_data;
use crate::route::HTTPS_CLIENT;
use crate::state::invocation_data::{
    take_logs, take_metrics, StoredLog, StoredMetric, StoredTrace,
};
use crate::util::parsers::parse_otlp_endpoint;

pub async fn flush_traces() {
    let traces = crate::state::invocation_entry::take_all_traces();
    send_traces(traces).await;
}

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;

use opentelemetry_proto::tonic::logs::v1::{ResourceLogs, ScopeLogs};

pub async fn flush_logs(exclude_invocation_id: Option<&str>) {
    flush_otlp_logs().await;
    flush_telemetry_logs(exclude_invocation_id).await;
    flush_metrics().await;
}

pub async fn flush_telemetry_logs(exclude_invocation_id: Option<&str>) {
    let logs = crate::state::invocation_entry::take_all_telemetry_logs(exclude_invocation_id);

    if logs.is_empty() {
        return;
    }

    let log_records = map_logs_to_otlp(&logs);

    if log_records.is_empty() {
        return;
    }

    let resource_logs = ResourceLogs {
        resource: Some(opentelemetry_proto::tonic::resource::v1::Resource {
            attributes: get_resources_attributes(),
            dropped_attributes_count: 0,
            ..Default::default()
        }),
        scope_logs: vec![ScopeLogs {
            scope: Some(opentelemetry_proto::tonic::common::v1::InstrumentationScope {
                name: "dash0.lambda-extension".to_string(),
                version: "1.0".to_string(),
                ..Default::default()
            }),
            log_records,
            ..Default::default()
        }],
        ..Default::default()
    };

    let export_request = ExportLogsServiceRequest {
        resource_logs: vec![resource_logs],
    };

    let body = export_request.encode_to_vec();

    let _ = send_request("/v1/logs", hyper::Method::POST, body, logs.len(), "logs").await;
}

pub async fn flush_otlp_logs() {
    let logs = take_logs();

    if logs.is_empty() {
        return;
    }

    let log_count = logs.len();

    let body = match build_logs_request(logs) {
        Some(req) => req,
        None => return,
    };

    let _ = send_request(
        "/v1/logs",
        hyper::Method::POST,
        body,
        log_count,
        "otlp logs",
    )
    .await;
}

pub async fn flush_metrics() {
    let metrics = take_metrics();

    if metrics.is_empty() {
        return;
    }

    let metric_count = metrics.len();

    let body = match build_metrics_request(metrics) {
        Some(req) => req,
        None => return,
    };

    let _ = send_request(
        "/v1/metrics",
        hyper::Method::POST,
        body,
        metric_count,
        "otlp metrics",
    )
    .await;
}

fn build_metrics_request(metrics: Vec<StoredMetric>) -> Option<Vec<u8>> {
    let mut metrics_iter = metrics.into_iter();
    let base_metric = match metrics_iter.next() {
        Some(metric) => metric,
        None => {
            tracing::error!(
                "[{}] build_metrics_request called with empty metrics vector",
                crate::log_prefix()
            );
            return None;
        }
    };

    let combined_resource_metrics = combine_metrics(&base_metric, metrics_iter);

    if combined_resource_metrics.is_empty() {
        return None;
    }

    let combined_export = ExportMetricsServiceRequest {
        resource_metrics: combined_resource_metrics,
    };

    Some(combined_export.encode_to_vec())
}

fn combine_metrics(
    base_metric: &StoredMetric,
    metrics_iter: std::vec::IntoIter<StoredMetric>,
) -> Vec<opentelemetry_proto::tonic::metrics::v1::ResourceMetrics> {
    let mut combined_resource_metrics = Vec::new();

    let process_metric =
        |metric: &StoredMetric,
         combined: &mut Vec<opentelemetry_proto::tonic::metrics::v1::ResourceMetrics>| {
            let decoded = match ExportMetricsServiceRequest::decode(metric.body.as_slice()) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!(
                        "[{}] Failed to decode metric payload: {}",
                        crate::log_prefix(),
                        err
                    );
                    return;
                }
            };

            combined.extend(decoded.resource_metrics);
        };

    process_metric(base_metric, &mut combined_resource_metrics);

    for metric in metrics_iter {
        process_metric(&metric, &mut combined_resource_metrics);
    }

    combined_resource_metrics
}

fn build_logs_request(logs: Vec<StoredLog>) -> Option<Vec<u8>> {
    let mut logs_iter = logs.into_iter();
    let base_log = match logs_iter.next() {
        Some(log) => log,
        None => {
            tracing::error!(
                "[{}] build_logs_request called with empty logs vector",
                crate::log_prefix()
            );
            return None;
        }
    };

    let combined_resource_logs = combine_logs(&base_log, logs_iter);

    if combined_resource_logs.is_empty() {
        return None;
    }

    let combined_export = ExportLogsServiceRequest {
        resource_logs: combined_resource_logs,
    };

    Some(combined_export.encode_to_vec())
}

fn combine_logs(
    base_log: &StoredLog,
    logs_iter: std::vec::IntoIter<StoredLog>,
) -> Vec<opentelemetry_proto::tonic::logs::v1::ResourceLogs> {
    let mut combined_resource_logs = Vec::new();

    let process_log =
        |log: &StoredLog,
         combined: &mut Vec<opentelemetry_proto::tonic::logs::v1::ResourceLogs>| {
            let decoded = match ExportLogsServiceRequest::decode(log.body.as_slice()) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!(
                        "[{}] Failed to decode log payload: {}",
                        crate::log_prefix(),
                        err
                    );
                    return;
                }
            };

            combined.extend(decoded.resource_logs);
        };

    process_log(base_log, &mut combined_resource_logs);

    for log in logs_iter {
        process_log(&log, &mut combined_resource_logs);
    }

    combined_resource_logs
}

pub async fn send_traces(traces: Vec<StoredTrace>) {
    if traces.is_empty() {
        return;
    }
    let mut ready_traces = Vec::new();
    for mut trace in traces {
        if let Ok(mut decoded) = ExportTraceServiceRequest::decode(trace.body.as_slice()) {
            let modified = merge_telemetry_invocation_data(&mut decoded);
            if modified > 0 {
                trace.body = decoded.encode_to_vec();
            }
        }
        ready_traces.push(trace);
    }
    let traces = ready_traces;
    if traces.is_empty() {
        return;
    }

    let trace_count = traces.len();

    let body = match _build_traces_request(traces) {
        Some(req) => req,
        None => return,
    };

    let _ = send_request(
        "/v1/traces",
        hyper::Method::POST,
        body,
        trace_count,
        "buffered traces",
    )
    .await;
}

fn _build_traces_request(traces: Vec<StoredTrace>) -> Option<Vec<u8>> {
    let mut traces_iter = traces.into_iter();
    let base_trace = match traces_iter.next() {
        Some(trace) => trace,
        None => {
            tracing::error!(
                "[{}] _build_traces_request called with empty traces vector",
                crate::log_prefix()
            );
            return None;
        }
    };

    let combined_resource_spans = combine_traces(&base_trace, traces_iter);

    if combined_resource_spans.is_empty() {
        return None;
    }

    let combined_export = ExportTraceServiceRequest {
        resource_spans: combined_resource_spans,
    };

    Some(combined_export.encode_to_vec())
}

fn _build_otlp_request(
    path: &str,
    method: hyper::Method,
    body: Vec<u8>,
) -> Result<Request<Body>, String> {
    let (scheme, authority) =
        parse_otlp_endpoint().ok_or_else(|| "Failed to parse OTLP endpoint".to_string())?;

    let target_uri = Uri::builder()
        .scheme(scheme.as_str())
        .authority(authority.as_str())
        .path_and_query(path)
        .build()
        .map_err(|err| format!("Failed building URI: {}", err))?;

    let mut builder = Request::builder().method(method).uri(target_uri);

    if let Some(headers) = builder.headers_mut() {
        if let Ok(host_val) = header::HeaderValue::from_str(&authority) {
            headers.insert(header::HOST, host_val);
        }

        if let Ok(len_val) = header::HeaderValue::from_str(&body.len().to_string()) {
            headers.insert(header::CONTENT_LENGTH, len_val);
        }

        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/x-protobuf"),
        );

        if let Ok(token) = std::env::var("DASH0_TOKEN") {
            if !token.is_empty() {
                if let Ok(auth_val) =
                    header::HeaderValue::from_str(format!("Bearer {}", token).as_str())
                {
                    headers.insert(header::AUTHORIZATION, auth_val);
                }
            }
        }
    } else {
        return Err("Failed to access headers for request; skipping".to_string());
    }

    builder
        .body(Body::from(body))
        .map_err(|err| format!("Failed to build request: {}", err))
}

async fn send_request(
    path: &str,
    method: hyper::Method,
    body: Vec<u8>,
    item_count: usize,
    item_type: &str,
) -> Result<(), ()> {
    let max_attempts = request_retries() + 1;

    for attempt in 1..=max_attempts {
        let req = match _build_otlp_request(path, method.clone(), body.clone()) {
            Ok(req) => req,
            Err(err) => {
                tracing::error!(
                    "[{}] Failed to build {} request: {}",
                    crate::log_prefix(),
                    item_type,
                    err
                );
                return Err(());
            }
        };

        let client = &*HTTPS_CLIENT;
        let start = std::time::Instant::now();
        match tokio::time::timeout(
            Duration::from_millis(request_timeout_ms()),
            client.request(req),
        )
        .await
        {
            Ok(Ok(resp)) => {
                if resp.status().is_success() {
                    tracing::info!(
                        count = item_count,
                        duration = start.elapsed().as_millis(),
                        "[{}] Sent {} (count={}) in {} ms, status={}",
                        crate::log_prefix(),
                        item_type,
                        item_count,
                        start.elapsed().as_millis(),
                        resp.status()
                    );
                    return Ok(());
                } else {
                    tracing::error!(
                        "[{}] Error sending {} Non-2xx sending {} in {} ms: status={} (attempt {}/{})",
                        crate::log_prefix(),
                        item_type,
                        item_type,
                        start.elapsed().as_millis(),
                        resp.status(),
                        attempt,
                        max_attempts
                    );
                }
            }
            Ok(Err(err)) => {
                tracing::error!(
                    "[{}] Error sending {} in {} ms: {} (attempt {}/{})",
                    crate::log_prefix(),
                    item_type,
                    start.elapsed().as_millis(),
                    err,
                    attempt,
                    max_attempts
                );
            }
            Err(_) => {
                tracing::error!(
                    "[{}] Error sending {} in {} ms: timeout (attempt {}/{})",
                    crate::log_prefix(),
                    item_type,
                    start.elapsed().as_millis(),
                    attempt,
                    max_attempts
                );
            }
        }
    }

    Err(())
}

fn combine_traces(
    base_trace: &StoredTrace,
    traces_iter: std::vec::IntoIter<StoredTrace>,
) -> Vec<opentelemetry_proto::tonic::trace::v1::ResourceSpans> {
    let mut combined_resource_spans = Vec::new();

    let process_trace =
        |trace: &StoredTrace,
         combined: &mut Vec<opentelemetry_proto::tonic::trace::v1::ResourceSpans>| {
            let decoded = match ExportTraceServiceRequest::decode(trace.body.as_slice()) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!(
                        "[{}] Failed to decode trace payload: {}",
                        crate::log_prefix(),
                        err
                    );
                    return;
                }
            };

            combined.extend(decoded.resource_spans);
        };

    process_trace(base_trace, &mut combined_resource_spans);

    for trace in traces_iter {
        process_trace(&trace, &mut combined_resource_spans);
    }

    combined_resource_spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::Method;
    use opentelemetry_proto::tonic::trace::v1::ResourceSpans;

    fn create_valid_trace(invocation_ids: Vec<String>, resource_spans_count: usize) -> StoredTrace {
        let resource_spans: Vec<ResourceSpans> = (0..resource_spans_count)
            .map(|_| ResourceSpans::default())
            .collect();

        let export_request = ExportTraceServiceRequest { resource_spans };

        StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: header::HeaderMap::new(),
            body: export_request.encode_to_vec(),
            invocation_ids,
        }
    }

    fn create_invalid_trace(invocation_ids: Vec<String>) -> StoredTrace {
        StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: header::HeaderMap::new(),
            body: vec![0xFF, 0xFF, 0xFF], // Invalid protobuf
            invocation_ids,
        }
    }

    #[test]
    fn test_combine_traces_single_valid_trace() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 2);
        let traces_iter = vec![].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 2);
    }

    #[test]
    fn test_combine_traces_multiple_valid_traces() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 2);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 3);
        let trace3 = create_valid_trace(vec!["inv-3".to_string()], 1);
        let traces_iter = vec![trace2, trace3].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 6); // 2 + 3 + 1
    }

    #[test]
    fn test_combine_traces_with_invalid_base_trace() {
        let base_trace = create_invalid_trace(vec!["inv-1".to_string()]);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 2);
        let traces_iter = vec![trace2].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 2); // Only from trace2
    }

    #[test]
    fn test_combine_traces_with_invalid_subsequent_trace() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 2);
        let trace2 = create_invalid_trace(vec!["inv-2".to_string()]);
        let trace3 = create_valid_trace(vec!["inv-3".to_string()], 1);
        let traces_iter = vec![trace2, trace3].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 3); // 2 from base + 1 from trace3
    }

    #[test]
    fn test_combine_traces_all_invalid() {
        let base_trace = create_invalid_trace(vec!["inv-1".to_string()]);
        let trace2 = create_invalid_trace(vec!["inv-2".to_string()]);
        let traces_iter = vec![trace2].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert!(resource_spans.is_empty());
    }

    #[test]
    fn test_combine_traces_with_multiple_invocation_ids() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string(), "inv-2".to_string()], 1);
        let trace2 = create_valid_trace(vec!["inv-3".to_string(), "inv-4".to_string()], 1);
        let traces_iter = vec![trace2].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 2);
    }

    #[test]
    fn test_combine_traces_empty_resource_spans() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 0);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 0);
        let traces_iter = vec![trace2].into_iter();

        let resource_spans = combine_traces(&base_trace, traces_iter);

        assert!(resource_spans.is_empty());
    }

    #[test]
    fn test_build_traces_request_happy_flow() {
        use std::env;

        // Set up environment variable for endpoint
        env::set_var("DASH0_ENDPOINT", "https://example.com:443/v1/traces");

        // Create valid traces
        let trace1 = create_valid_trace(vec!["inv-1".to_string()], 2);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 3);
        let traces = vec![trace1, trace2];

        // Call _build_traces_request
        let result = _build_traces_request(traces);

        // Verify we got a result
        assert!(result.is_some());
    }
}

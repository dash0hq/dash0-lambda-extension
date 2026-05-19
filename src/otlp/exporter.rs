use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{header, Request, Uri};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::config::{get_dash0_dataset, request_retries, request_timeout_ms};
use crate::otlp::log_mutations::map_logs_to_otlp;
use crate::otlp::resources::get_resources_attributes;
use crate::route::{ReqBody, HTTPS_CLIENT};
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
            scope: Some(
                opentelemetry_proto::tonic::common::v1::InstrumentationScope {
                    name: "dash0.lambda-extension".to_string(),
                    version: "1.0".to_string(),
                    ..Default::default()
                },
            ),
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
    let combined_resource_metrics = combine_items(
        &metrics,
        |m| &m.body,
        |b| ExportMetricsServiceRequest::decode(b).map(|d| d.resource_metrics),
        "metric",
    );

    if combined_resource_metrics.is_empty() {
        return None;
    }

    let combined_export = ExportMetricsServiceRequest {
        resource_metrics: combined_resource_metrics,
    };

    Some(combined_export.encode_to_vec())
}

fn build_logs_request(logs: Vec<StoredLog>) -> Option<Vec<u8>> {
    let combined_resource_logs = combine_items(
        &logs,
        |l| &l.body,
        |b| ExportLogsServiceRequest::decode(b).map(|d| d.resource_logs),
        "log",
    );

    if combined_resource_logs.is_empty() {
        return None;
    }

    let combined_export = ExportLogsServiceRequest {
        resource_logs: combined_resource_logs,
    };

    Some(combined_export.encode_to_vec())
}

pub async fn send_traces(traces: Vec<StoredTrace>) {
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
    let mut combined_resource_spans = combine_items(
        &traces,
        |t| &t.body,
        |b| ExportTraceServiceRequest::decode(b).map(|d| d.resource_spans),
        "trace",
    );

    if combined_resource_spans.is_empty() {
        return None;
    }

    add_env_vars(&mut combined_resource_spans);

    let combined_export = ExportTraceServiceRequest {
        resource_spans: combined_resource_spans,
    };

    Some(combined_export.encode_to_vec())
}

fn _build_otlp_request(
    path: &str,
    method: hyper::Method,
    body: Vec<u8>,
) -> Result<Request<ReqBody>, String> {
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

        if let Some(token) = crate::config::get_dash0_token() {
            if !token.is_empty() {
                if let Ok(auth_val) =
                    header::HeaderValue::from_str(format!("Bearer {}", token).as_str())
                {
                    headers.insert(header::AUTHORIZATION, auth_val);
                }
            }
        }

        if let Some(dataset) = get_dash0_dataset() {
            if let Ok(dataset_val) = header::HeaderValue::from_str(&dataset) {
                headers.insert(
                    header::HeaderName::from_static("dash0-dataset"),
                    dataset_val,
                );
            }
        }
    } else {
        return Err("Failed to access headers for request; skipping".to_string());
    }

    builder
        .body(Full::new(Bytes::from(body)))
        .map_err(|err| format!("Failed to build request: {}", err))
}

async fn send_request(
    path: &str,
    method: hyper::Method,
    body: Vec<u8>,
    item_count: usize,
    item_type: &str,
) -> Result<(), ()> {
    if crate::config::get_dash0_token().is_none() {
        return Ok(());
    }

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

fn add_env_vars(resource_spans: &mut [opentelemetry_proto::tonic::trace::v1::ResourceSpans]) {
    let env_attrs = crate::state::global::get_env_var_attrs();
    if env_attrs.is_empty() {
        return;
    }
    for rs in resource_spans.iter_mut() {
        let resource = rs.resource.get_or_insert_with(Default::default);
        resource.attributes.extend(env_attrs.clone());
    }
}

fn combine_items<S, R>(
    items: &[S],
    get_body: impl Fn(&S) -> &[u8],
    decode_and_extract: impl Fn(&[u8]) -> Result<Vec<R>, prost::DecodeError>,
    item_type: &str,
) -> Vec<R> {
    let mut combined = Vec::new();
    for item in items {
        match decode_and_extract(get_body(item)) {
            Ok(resources) => combined.extend(resources),
            Err(err) => {
                tracing::error!(
                    "[{}] Failed to decode {} payload: {}",
                    crate::log_prefix(),
                    item_type,
                    err
                );
            }
        }
    }
    combined
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

    fn combine_test_traces(traces: &[StoredTrace]) -> Vec<ResourceSpans> {
        combine_items(
            traces,
            |t| &t.body,
            |b| ExportTraceServiceRequest::decode(b).map(|d| d.resource_spans),
            "trace",
        )
    }

    #[test]
    fn test_combine_traces_single_valid_trace() {
        let traces = vec![create_valid_trace(vec!["inv-1".to_string()], 2)];

        let resource_spans = combine_test_traces(&traces);

        assert_eq!(resource_spans.len(), 2);
    }

    #[test]
    fn test_combine_traces_multiple_valid_traces() {
        let traces = vec![
            create_valid_trace(vec!["inv-1".to_string()], 2),
            create_valid_trace(vec!["inv-2".to_string()], 3),
            create_valid_trace(vec!["inv-3".to_string()], 1),
        ];

        let resource_spans = combine_test_traces(&traces);

        assert_eq!(resource_spans.len(), 6); // 2 + 3 + 1
    }

    #[test]
    fn test_combine_traces_with_invalid_base_trace() {
        let traces = vec![
            create_invalid_trace(vec!["inv-1".to_string()]),
            create_valid_trace(vec!["inv-2".to_string()], 2),
        ];

        let resource_spans = combine_test_traces(&traces);

        assert_eq!(resource_spans.len(), 2); // Only from trace2
    }

    #[test]
    fn test_combine_traces_with_invalid_subsequent_trace() {
        let traces = vec![
            create_valid_trace(vec!["inv-1".to_string()], 2),
            create_invalid_trace(vec!["inv-2".to_string()]),
            create_valid_trace(vec!["inv-3".to_string()], 1),
        ];

        let resource_spans = combine_test_traces(&traces);

        assert_eq!(resource_spans.len(), 3); // 2 from base + 1 from trace3
    }

    #[test]
    fn test_combine_traces_all_invalid() {
        let traces = vec![
            create_invalid_trace(vec!["inv-1".to_string()]),
            create_invalid_trace(vec!["inv-2".to_string()]),
        ];

        let resource_spans = combine_test_traces(&traces);

        assert!(resource_spans.is_empty());
    }

    #[test]
    fn test_combine_traces_with_multiple_invocation_ids() {
        let traces = vec![
            create_valid_trace(vec!["inv-1".to_string(), "inv-2".to_string()], 1),
            create_valid_trace(vec!["inv-3".to_string(), "inv-4".to_string()], 1),
        ];

        let resource_spans = combine_test_traces(&traces);

        assert_eq!(resource_spans.len(), 2);
    }

    #[test]
    fn test_combine_traces_empty_resource_spans() {
        let traces = vec![
            create_valid_trace(vec!["inv-1".to_string()], 0),
            create_valid_trace(vec!["inv-2".to_string()], 0),
        ];

        let resource_spans = combine_test_traces(&traces);

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

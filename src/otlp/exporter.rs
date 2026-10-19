use std::io::Write;
use std::time::Duration;

use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use http_body_util::Full;
use hyper::{header, Request, Uri};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::resource::v1::Resource;
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
            attributes: lambda_resource_attributes(),
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
    let mut combined_resource_metrics = combine_items(
        &metrics,
        |m| &m.body,
        |b| ExportMetricsServiceRequest::decode(b).map(|d| d.resource_metrics),
        "metric",
    );

    if combined_resource_metrics.is_empty() {
        return None;
    }

    let attributes = lambda_resource_attributes();
    for rm in combined_resource_metrics.iter_mut() {
        merge_resource_attributes(&mut rm.resource, &attributes);
    }

    let combined_export = ExportMetricsServiceRequest {
        resource_metrics: combined_resource_metrics,
    };

    Some(combined_export.encode_to_vec())
}

fn build_logs_request(logs: Vec<StoredLog>) -> Option<Vec<u8>> {
    let mut combined_resource_logs = combine_items(
        &logs,
        |l| &l.body,
        |b| ExportLogsServiceRequest::decode(b).map(|d| d.resource_logs),
        "log",
    );

    if combined_resource_logs.is_empty() {
        return None;
    }

    let attributes = lambda_resource_attributes();
    for rl in combined_resource_logs.iter_mut() {
        merge_resource_attributes(&mut rl.resource, &attributes);
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

    let attributes = lambda_resource_attributes();
    for rs in combined_resource_spans.iter_mut() {
        merge_resource_attributes(&mut rs.resource, &attributes);
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
    gzip_encoded: bool,
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

        if gzip_encoded {
            headers.insert(
                header::CONTENT_ENCODING,
                header::HeaderValue::from_static("gzip"),
            );
        }

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

    // Compress once, before the retry loop: re-gzipping the identical payload on
    // every attempt would waste CPU during the post-invocation flush window.
    let gzip_encoded = crate::config::is_compression_enabled();
    let body = if gzip_encoded {
        match gzip(&body) {
            Ok(compressed) => compressed,
            Err(err) => {
                tracing::error!(
                    "[{}] Failed to gzip {} body: {}",
                    crate::log_prefix(),
                    item_type,
                    err
                );
                return Err(());
            }
        }
    } else {
        body
    };

    let max_attempts = request_retries() + 1;

    for attempt in 1..=max_attempts {
        let req = match _build_otlp_request(path, method.clone(), body.clone(), gzip_encoded) {
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

fn gzip(body: &[u8]) -> std::io::Result<Vec<u8>> {
    // Level 3 keeps most of the size win for protobuf payloads at noticeably
    // less CPU than the default (6); CPU is the scarcer resource during the
    // post-invocation flush window.
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(3));
    encoder.write_all(body)?;
    encoder.finish()
}

/// The resource attributes every signal leaving the extension carries: the ones
/// the extension puts on its own telemetry, plus the function's environment
/// variables as `process.environment_variable.*`.
fn lambda_resource_attributes() -> Vec<KeyValue> {
    let mut attributes = get_resources_attributes();
    merge_missing_attributes(&mut attributes, &crate::state::global::get_env_var_attrs());
    attributes
}

/// Adds the Lambda resource attributes to a resource received from user code.
/// Keys the SDK already set are kept, so the SDK's own values win. Without this,
/// SDK telemetry and the extension's own telemetry for the same function can
/// land on different resources.
fn merge_resource_attributes(resource: &mut Option<Resource>, attributes: &[KeyValue]) {
    let resource = resource.get_or_insert_with(Default::default);
    merge_missing_attributes(&mut resource.attributes, attributes);
}

fn merge_missing_attributes(target: &mut Vec<KeyValue>, attributes: &[KeyValue]) {
    for kv in attributes {
        if !target.iter().any(|existing| existing.key == kv.key) {
            target.push(kv.clone());
        }
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

    #[tokio::test]
    async fn test_build_otlp_request_gzip_wiring() {
        use flate2::read::GzDecoder;
        use http_body_util::BodyExt;
        use std::io::Read;

        std::env::set_var("DASH0_ENDPOINT", "https://example.com:443/v1/traces");

        let original = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans::default()],
        }
        .encode_to_vec();
        let compressed = gzip(&original).expect("gzip should succeed");

        let req = _build_otlp_request("/v1/traces", Method::POST, compressed.clone(), true)
            .expect("request should build");

        // (a) Content-Encoding: gzip is set
        assert_eq!(
            req.headers()
                .get(header::CONTENT_ENCODING)
                .unwrap()
                .to_str()
                .unwrap(),
            "gzip"
        );
        // (b) Content-Length matches the compressed body length
        assert_eq!(
            req.headers()
                .get(header::CONTENT_LENGTH)
                .unwrap()
                .to_str()
                .unwrap(),
            compressed.len().to_string()
        );

        // (c) The body is sent verbatim and gunzips back to the original protobuf.
        // Also guards against reordering the gzip call relative to header setup.
        let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body_bytes.as_ref(), compressed.as_slice());
        let mut decoder = GzDecoder::new(&body_bytes[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[tokio::test]
    async fn test_build_otlp_request_without_gzip() {
        use http_body_util::BodyExt;

        std::env::set_var("DASH0_ENDPOINT", "https://example.com:443/v1/traces");

        let body = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans::default()],
        }
        .encode_to_vec();

        let req = _build_otlp_request("/v1/traces", Method::POST, body.clone(), false)
            .expect("request should build");

        // No Content-Encoding header when compression is disabled
        assert!(req.headers().get(header::CONTENT_ENCODING).is_none());
        // Content-Length matches the raw body, which is sent verbatim
        assert_eq!(
            req.headers()
                .get(header::CONTENT_LENGTH)
                .unwrap()
                .to_str()
                .unwrap(),
            body.len().to_string()
        );
        let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body_bytes.as_ref(), body.as_slice());
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

    fn string_kv(key: &str, value: &str) -> KeyValue {
        use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue};
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(value.to_string())),
            }),
        }
    }

    fn attribute_values(resource: &Option<Resource>, key: &str) -> Vec<String> {
        use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue};
        resource
            .as_ref()
            .map(|r| r.attributes.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|kv| kv.key == key)
            .filter_map(|kv| match &kv.value {
                Some(AnyValue {
                    value: Some(Value::StringValue(s)),
                }) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    // A resource as an SDK sends it: OTEL_RESOURCE_ATTRIBUTES applied, but none of
    // the attributes only the extension knows, like the function ARN.
    fn sdk_resource() -> Option<Resource> {
        Some(Resource {
            attributes: vec![
                string_kv("service.name", "sdk-service"),
                string_kv("cloud.platform", "set-by-sdk"),
            ],
            ..Default::default()
        })
    }

    // Puts the process-wide state that get_resources_attributes and
    // get_env_var_attrs read into a known shape. Callers must be #[serial].
    fn set_up_lambda_state() {
        crate::state::global::reset_for_tests();
        let _ = std::fs::remove_file("/tmp/dash0_env_vars");
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");
        std::env::set_var("AWS_REGION", "eu-west-1");
        std::env::set_var("AWS_LAMBDA_FUNCTION_NAME", "my-fn");
        crate::state::global::store_account_id("123456789012");

        std::fs::write(
            "/tmp/dash0_env_vars",
            serde_json::json!({ "MY_SETTING": "on" }).to_string(),
        )
        .unwrap();
        crate::state::global::init_env_var_attrs();
        std::fs::remove_file("/tmp/dash0_env_vars").unwrap();
    }

    fn tear_down_lambda_state() {
        crate::state::global::reset_for_tests();
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
    }

    const FUNCTION_ARN: &str = "arn:aws:lambda:eu-west-1:123456789012:function:my-fn";

    fn assert_enriched(resource: &Option<Resource>) {
        assert_eq!(
            attribute_values(resource, "cloud.resource_id"),
            vec![FUNCTION_ARN]
        );
        assert_eq!(
            attribute_values(resource, "cloud.account.id"),
            vec!["123456789012"]
        );
        assert_eq!(
            attribute_values(resource, "process.environment_variable.MY_SETTING"),
            vec!["on"]
        );
        // The SDK's own values win, and no key appears twice.
        assert_eq!(
            attribute_values(resource, "service.name"),
            vec!["sdk-service"]
        );
        assert_eq!(
            attribute_values(resource, "cloud.platform"),
            vec!["set-by-sdk"]
        );
    }

    #[test]
    fn test_merge_resource_attributes_keeps_existing_keys() {
        let mut resource = sdk_resource();
        let attributes = vec![
            string_kv("service.name", "from-extension"),
            string_kv("cloud.resource_id", "arn"),
        ];

        merge_resource_attributes(&mut resource, &attributes);

        assert_eq!(
            attribute_values(&resource, "service.name"),
            vec!["sdk-service"]
        );
        assert_eq!(
            attribute_values(&resource, "cloud.resource_id"),
            vec!["arn"]
        );
        assert_eq!(resource.unwrap().attributes.len(), 3);
    }

    #[test]
    fn test_merge_resource_attributes_creates_missing_resource() {
        let mut resource = None;

        merge_resource_attributes(&mut resource, &[string_kv("cloud.platform", "aws_lambda")]);

        assert_eq!(
            attribute_values(&resource, "cloud.platform"),
            vec!["aws_lambda"]
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_build_metrics_request_adds_lambda_resource_attributes() {
        use opentelemetry_proto::tonic::metrics::v1::ResourceMetrics;
        set_up_lambda_state();

        let body = ExportMetricsServiceRequest {
            resource_metrics: vec![
                ResourceMetrics {
                    resource: sdk_resource(),
                    ..Default::default()
                },
                // Some exporters omit the resource entirely.
                ResourceMetrics::default(),
            ],
        }
        .encode_to_vec();
        let metric = StoredMetric {
            method: Method::POST,
            path_and_query: "/v1/metrics".to_string(),
            headers: header::HeaderMap::new(),
            body,
        };

        let encoded = build_metrics_request(vec![metric]).expect("request should build");
        tear_down_lambda_state();

        let decoded = ExportMetricsServiceRequest::decode(encoded.as_slice()).unwrap();
        assert_enriched(&decoded.resource_metrics[0].resource);
        let bare = &decoded.resource_metrics[1].resource;
        assert_eq!(attribute_values(bare, "cloud.platform"), vec!["aws_lambda"]);
        assert_eq!(
            attribute_values(bare, "cloud.resource_id"),
            vec![FUNCTION_ARN]
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_build_logs_request_adds_lambda_resource_attributes() {
        set_up_lambda_state();

        let body = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: sdk_resource(),
                ..Default::default()
            }],
        }
        .encode_to_vec();
        let log = StoredLog {
            method: Method::POST,
            path_and_query: "/v1/logs".to_string(),
            headers: header::HeaderMap::new(),
            body,
        };

        let encoded = build_logs_request(vec![log]).expect("request should build");
        tear_down_lambda_state();

        let decoded = ExportLogsServiceRequest::decode(encoded.as_slice()).unwrap();
        assert_enriched(&decoded.resource_logs[0].resource);
    }

    #[test]
    #[serial_test::serial]
    fn test_build_traces_request_adds_lambda_resource_attributes() {
        set_up_lambda_state();

        let body = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: sdk_resource(),
                ..Default::default()
            }],
        }
        .encode_to_vec();
        let trace = StoredTrace {
            method: Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: header::HeaderMap::new(),
            body,
            invocation_ids: vec!["inv-1".to_string()],
        };

        let encoded = _build_traces_request(vec![trace]).expect("request should build");
        tear_down_lambda_state();

        let decoded = ExportTraceServiceRequest::decode(encoded.as_slice()).unwrap();
        assert_enriched(&decoded.resource_spans[0].resource);
    }
}

use std::io::Write;
use std::time::Duration;

use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use http_body_util::Full;
use hyper::{header, Request, Uri};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::config::{get_dash0_dataset, request_retries, request_timeout_ms};
use crate::otlp::log_mutations::map_logs_to_otlp;
use crate::otlp::payload_masking::PayloadRules;
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
    let mut logs = crate::state::invocation_entry::take_all_telemetry_logs(exclude_invocation_id);

    if logs.is_empty() {
        return;
    }

    if let Some(rules) = crate::otlp::payload_masking::rules() {
        mask_http_payload_logs(&mut logs, rules);
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

    let body = match build_logs_request(logs, crate::otlp::payload_masking::rules()) {
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

fn build_logs_request(logs: Vec<StoredLog>, rules: Option<&PayloadRules>) -> Option<Vec<u8>> {
    let mut combined_resource_logs = combine_items(
        &logs,
        |l| &l.body,
        |b| ExportLogsServiceRequest::decode(b).map(|d| d.resource_logs),
        "log",
    );

    if combined_resource_logs.is_empty() {
        return None;
    }

    if let Some(rules) = rules {
        crate::otlp::payload_masking::mask_resource_logs(&mut combined_resource_logs, rules);
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

    let body = match _build_traces_request(traces, crate::otlp::payload_masking::rules()) {
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

fn _build_traces_request(
    traces: Vec<StoredTrace>,
    rules: Option<&PayloadRules>,
) -> Option<Vec<u8>> {
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
    if let Some(rules) = rules {
        crate::otlp::payload_masking::mask_resource_spans(&mut combined_resource_spans, rules);
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

/// HTTP bodies are moved off their spans into `dash0_payload` logs on receive; mask them here,
/// at export, so the receive handler that the distro's flush waits on does no masking work.
fn mask_http_payload_logs(
    logs: &mut [crate::state::invocation_data::TelemetryLog],
    rules: &PayloadRules,
) {
    use crate::otlp::payload_masking::{mask_payload_log_body, Kind};
    for log in logs.iter_mut() {
        let kind = match log.custom_attributes.iter().find_map(|(k, v)| {
            (k == crate::otlp::attributes::DASH0_FAAS_PAYLOAD_TYPE).then_some(v.as_str())
        }) {
            Some("http_request_body") => Kind::RequestBody,
            Some("http_response_body") => Kind::ResponseBody,
            _ => continue,
        };
        if let serde_json::Value::String(body) = &mut log.record {
            if let std::borrow::Cow::Owned(masked) = mask_payload_log_body(body, rules.rule(kind)) {
                *body = masked;
            }
        }
    }
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
        let result = _build_traces_request(traces, None);

        // Verify we got a result
        assert!(result.is_some());
    }
}

/// Receive → store → export build for one invocation, per `DASH0_MASK_*` configuration,
/// asserting on the bytes the exporter would send. The binary-level harness is
/// `tests/masking_e2e.rs`.
#[cfg(test)]
mod pipeline {
    use super::*;
    use crate::otlp::payload_masking::MASKED;
    use crate::otlp::span_mutations::process_trace_request;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use serial_test::serial;

    const S: &str = "XQZSECRET";

    fn kv(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(value.into())),
            }),
        }
    }

    struct Exported {
        attrs: Vec<(String, String)>,
        request_body_log: String,
        response_body_log: String,
    }

    impl Exported {
        fn attr(&self, key: &str) -> &str {
            &self.attrs.iter().find(|(k, _)| k == key).unwrap().1
        }
    }

    fn export(vars: &[(&str, &str)], enabled: bool) -> Exported {
        let rules = PayloadRules::from_lookup(|k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        });
        let id = "pipeline-masking-1";
        crate::state::invocation_entry::remove(id);
        crate::state::invocation_data::store_current_invocation_id(id);

        let mut request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "@opentelemetry/instrumentation-http".into(),
                        ..Default::default()
                    }),
                    spans: vec![Span {
                        trace_id: vec![1; 16],
                        span_id: vec![2; 8],
                        kind: 3,
                        start_time_unix_nano: 10_000_000,
                        end_time_unix_nano: 20_000_000,
                        attributes: vec![
                            kv(
                                "url.full",
                                &format!("https://h/p?token={S}&email=jane&page=1"),
                            ),
                            kv(
                                "http.request.query",
                                &format!(r#"{{"password":"{S}","email":"jane"}}"#),
                            ),
                            kv(
                                "http.request.headers",
                                &format!(r#"{{"authorization":"{S}","x-email":"jane"}}"#),
                            ),
                            kv(
                                "http.response.headers",
                                &format!(r#"{{"x-api-key":"{S}","x-email":"jane"}}"#),
                            ),
                            kv("http.request.header.authorization", S),
                            kv(
                                "http.request.body",
                                &format!(r#"{{"password":"{S}","email":"jane","orderId":"42"}}"#),
                            ),
                            // Cut by the distro's attribute limit.
                            kv(
                                "http.response.body",
                                &format!(r#"{{"email":"jane","sessionToken":"{S}"#),
                            ),
                            kv("gen_ai.usage.input_tokens", "12"),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut ids = Vec::new();
        let mut body = Vec::new();
        process_trace_request(&mut request, &mut ids, &mut body);

        let trace = StoredTrace {
            method: hyper::Method::POST,
            path_and_query: "/v1/traces".into(),
            headers: Default::default(),
            body,
            invocation_ids: vec![id.into()],
        };
        let rules = enabled.then_some(&rules);
        let exported = ExportTraceServiceRequest::decode(
            _build_traces_request(vec![trace], rules)
                .unwrap()
                .as_slice(),
        )
        .unwrap();

        let mut logs = crate::state::invocation_entry::remove(id).unwrap().logs;
        if let Some(rules) = rules {
            mask_http_payload_logs(&mut logs, rules);
        }
        let log_of = |t: &str| {
            logs.iter()
                .find(|l| l.custom_attributes.iter().any(|(_, v)| v == t))
                .and_then(|l| l.record.as_str().map(str::to_string))
                .unwrap()
        };
        Exported {
            attrs: exported.resource_spans[0].scope_spans[0].spans[0]
                .attributes
                .iter()
                .filter_map(|kv| match &kv.value {
                    Some(AnyValue {
                        value: Some(Value::StringValue(v)),
                    }) => Some((kv.key.clone(), v.clone())),
                    _ => None,
                })
                .collect(),
            request_body_log: log_of("http_request_body"),
            response_body_log: log_of("http_response_body"),
        }
    }

    #[test]
    #[serial]
    fn disabled_exports_payloads_as_received() {
        let e = export(&[], false);
        assert!(e.attr("url.full").contains(S));
        assert!(e.attr("http.request.header.authorization").contains(S));
        assert!(e.request_body_log.contains(S));
    }

    #[test]
    #[serial]
    fn defaults_mask_every_payload_and_nothing_else() {
        let e = export(&[], true);
        for (k, v) in &e.attrs {
            assert!(!v.contains(S), "{k} leaked: {v}");
        }
        assert!(!e.request_body_log.contains(S), "{}", e.request_body_log);
        assert!(!e.response_body_log.contains(S), "{}", e.response_body_log);
        assert_eq!(
            e.attr("url.full"),
            "https://h/p?token=****&email=jane&page=1"
        );
        assert_eq!(e.attr("gen_ai.usage.input_tokens"), "12");
        // Personal data the defaults don't name is kept.
        assert!(e.request_body_log.contains(r#""email":"jane""#));
        assert!(e.attr("http.request.headers").contains("jane"));
    }

    #[test]
    #[serial]
    fn custom_mask_rules_replace_the_defaults_everywhere() {
        let e = export(&[("DASH0_MASK_RULES", r#"[".*email.*"]"#)], true);
        for (k, v) in &e.attrs {
            assert!(!v.contains("jane"), "{k} kept personal data: {v}");
        }
        assert!(
            e.attr("url.full").contains(S),
            "defaults must no longer apply"
        );
        assert!(!e.request_body_log.contains("jane") && e.request_body_log.contains(S));
    }

    #[test]
    #[serial]
    fn each_variable_controls_only_its_payload() {
        let cases: &[(&str, &str)] = &[
            ("DASH0_MASK_QUERY_PARAMS", "url.full"),
            ("DASH0_MASK_QUERY_PARAMS", "http.request.query"),
            ("DASH0_MASK_REQUEST_HEADERS", "http.request.headers"),
            (
                "DASH0_MASK_REQUEST_HEADERS",
                "http.request.header.authorization",
            ),
            ("DASH0_MASK_RESPONSE_HEADERS", "http.response.headers"),
        ];
        for (var, key) in cases {
            // `[]` turns masking off for this payload type only.
            let e = export(&[(var, "[]")], true);
            assert!(e.attr(key).contains(S), "{var}=[] should leave {key}");
            for (k, v) in &e.attrs {
                if !cases.iter().any(|(cv, ck)| cv == var && ck == k) {
                    assert!(!v.contains(S), "{var}=[] unmasked {k}: {v}");
                }
            }
        }
        let e = export(&[("DASH0_MASK_REQUEST_BODY", "[]")], true);
        assert!(e.request_body_log.contains(S));
        assert!(!e.response_body_log.contains(S));
        let e = export(&[("DASH0_MASK_RESPONSE_BODY", "[]")], true);
        assert!(!e.request_body_log.contains(S));
        assert!(e.response_body_log.contains(S));
    }

    #[test]
    #[serial]
    fn all_hides_whole_payloads() {
        let e = export(
            &[
                ("DASH0_MASK_REQUEST_BODY", "all"),
                ("DASH0_MASK_REQUEST_HEADERS", "all"),
                ("DASH0_MASK_QUERY_PARAMS", "all"),
            ],
            true,
        );
        assert_eq!(e.attr("http.request.headers"), MASKED);
        assert_eq!(e.attr("http.request.query"), MASKED);
        assert_eq!(e.attr("url.full"), "https://h/p?****");
        assert!(
            e.request_body_log.starts_with(r#"{"message":"****""#),
            "{}",
            e.request_body_log
        );
        // DASH0_MASK_RULES=all reaches every payload type that has no own variable.
        let e = export(&[("DASH0_MASK_RULES", "all")], true);
        assert_eq!(e.attr("http.response.headers"), MASKED);
    }

    #[test]
    #[serial]
    fn invalid_values_fall_back_to_mask_rules() {
        let e = export(
            &[
                ("DASH0_MASK_REQUEST_HEADERS", "{not json"),
                ("DASH0_MASK_RULES", r#"[".*email.*"]"#),
            ],
            true,
        );
        assert!(!e.attr("http.request.headers").contains("jane"));
        assert!(e.attr("http.request.headers").contains(S));
    }
}

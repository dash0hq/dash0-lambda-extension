use std::collections::HashSet;
use std::time::Duration;

use hyper::{header, Body, Request, Uri};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;

use crate::route::HTTPS_CLIENT;
use crate::store::{store_traces, take_telemetry_logs, take_traces, StoredTrace};
use crate::util::log_mutations::{get_resources_attributes, map_logs_to_otlp};
use crate::util::parsers::parse_otlp_endpoint;
use crate::util::span_mutations::merge_telemetry_invocation_data;

pub async fn flush_traces() {
    let traces = take_traces();
    if traces.is_empty() {
        return;
    }

    send_traces(traces).await;
}

use crate::store::store_telemetry_logs;

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;

use opentelemetry_proto::tonic::logs::v1::{ResourceLogs, ScopeLogs};

pub async fn flush_logs(is_invocation_end: bool) {
    let logs = take_telemetry_logs();

    if logs.is_empty() {
        return;
    }

    let log_records = map_logs_to_otlp(&logs, is_invocation_end);

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
            log_records,
            ..Default::default()
        }],
        ..Default::default()
    };

    let export_request = ExportLogsServiceRequest {
        resource_logs: vec![resource_logs],
    };

    let body = export_request.encode_to_vec();

    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/x-protobuf"),
    );

    let req = match _build_otlp_request("/v1/logs", hyper::Method::POST, body, Some(&headers)) {
        Ok(req) => req,
        Err(err) => {
            tracing::error!("[LRAP] Failed to build log request: {}", err);
            store_telemetry_logs(logs);
            return;
        }
    };

    let client = &*HTTPS_CLIENT;
    if let Err(_err) = send_request(client, req, logs.len(), "logs").await {
        store_telemetry_logs(logs);
    }
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

    let original_traces = traces.clone();

    let (req, combined_trace, mut failed) = match _build_traces_request(traces, &original_traces) {
        Some(result) => result,
        None => return,
    };

    // match ExportTraceServiceRequest::decode(combined_trace.body.as_slice()) {
    //     Ok(decoded_trace) => {
    //         tracing::info!("[LRAP] Combined trace payload: {:?}", decoded_trace);
    //     }
    //     Err(err) => {
    //         tracing::error!("[LRAP] Failed to decode combined trace payload: {}", err);
    //     }
    // }

    let client = &*HTTPS_CLIENT;

    if let Err(_err) = send_request(client, req, original_traces.len(), "buffered traces").await {
        failed.extend(original_traces.clone());
    }

    if !failed.is_empty() {
        store_traces(failed)
    }

    let mut seen = HashSet::new();
    for id in combined_trace.invocation_ids {
        if seen.insert(id.clone()) {
            crate::store::cleanup_invocation(&id);
        }
    }
}

fn _build_traces_request(
    traces: Vec<StoredTrace>,
    original_traces: &[StoredTrace],
) -> Option<(Request<Body>, StoredTrace, Vec<StoredTrace>)> {
    let mut traces_iter = traces.into_iter();
    let base_trace = traces_iter.next().expect("traces not empty");

    let (combined_resource_spans, all_invocation_ids, failed) =
        combine_traces(&base_trace, traces_iter);

    if combined_resource_spans.is_empty() {
        store_traces(original_traces.to_vec());
        return None;
    }

    let combined_export = ExportTraceServiceRequest {
        resource_spans: combined_resource_spans,
    };

    let combined_trace = StoredTrace {
        method: base_trace.method.clone(),
        path_and_query: base_trace.path_and_query.clone(),
        headers: base_trace.headers.clone(),
        body: combined_export.encode_to_vec(),
        invocation_ids: all_invocation_ids,
    };

    let req = match _build_otlp_request(
        combined_trace.path_and_query.as_str(),
        combined_trace.method.clone(),
        combined_trace.body.clone(),
        Some(&combined_trace.headers),
    ) {
        Ok(req) => req,
        Err(err) => {
            tracing::error!("[LRAP] Failed to build trace request: {}", err);
            store_traces(original_traces.to_vec());
            return None;
        }
    };

    Some((req, combined_trace, failed))
}

fn _build_otlp_request(
    path: &str,
    method: hyper::Method,
    body: Vec<u8>,
    headers_to_merge: Option<&header::HeaderMap>,
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
        if let Some(h_map) = headers_to_merge {
            for (k, v) in h_map.iter() {
                if k == header::HOST || k == header::CONTENT_LENGTH {
                    continue;
                }
                headers.insert(k, v.clone());
            }
        }

        if let Ok(host_val) = header::HeaderValue::from_str(&authority) {
            headers.insert(header::HOST, host_val);
        }

        if let Ok(len_val) = header::HeaderValue::from_str(&body.len().to_string()) {
            headers.insert(header::CONTENT_LENGTH, len_val);
        }

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
    client: &hyper::Client<hyper_rustls::HttpsConnector<hyper::client::HttpConnector>>,
    req: Request<Body>,
    item_count: usize,
    item_type: &str,
) -> Result<(), ()> {
    let start = std::time::Instant::now();
    match tokio::time::timeout(Duration::from_secs(2), client.request(req)).await {
        Ok(Ok(resp)) => {
            if resp.status().is_success() {
                tracing::info!(
                    count = item_count,
                    duration = start.elapsed().as_millis(),
                    "[LRAP] Sent {} (count={}) in {} ms, status={}",
                    item_type,
                    item_count,
                    start.elapsed().as_millis(),
                    resp.status()
                );
                Ok(())
            } else {
                tracing::error!(
                    "[LRAP] Error sending {} Non-2xx sending {} in {} ms: status={}",
                    item_type,
                    item_type,
                    start.elapsed().as_millis(),
                    resp.status()
                );
                Err(())
            }
        }
        Ok(Err(err)) => {
            tracing::error!(
                "[LRAP] Error sending {} in {} ms: {}",
                item_type,
                start.elapsed().as_millis(),
                err
            );
            Err(())
        }
        Err(_) => {
            tracing::error!(
                "[LRAP] Error sending {} in {} ms: timeout",
                item_type,
                start.elapsed().as_millis()
            );
            Err(())
        }
    }
}

fn combine_traces(
    base_trace: &StoredTrace,
    traces_iter: std::vec::IntoIter<StoredTrace>,
) -> (
    Vec<opentelemetry_proto::tonic::trace::v1::ResourceSpans>,
    Vec<String>,
    Vec<StoredTrace>,
) {
    let mut combined_resource_spans = Vec::new();
    let mut all_invocation_ids = base_trace.invocation_ids.clone();
    let mut failed = Vec::new();

    let process_trace =
        |trace: &StoredTrace,
         combined: &mut Vec<opentelemetry_proto::tonic::trace::v1::ResourceSpans>,
         failed: &mut Vec<StoredTrace>| {
            let decoded = match ExportTraceServiceRequest::decode(trace.body.as_slice()) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!("[LRAP] Failed to decode trace payload: {}", err);
                    failed.push(trace.clone());
                    return;
                }
            };

            combined.extend(decoded.resource_spans);
        };

    process_trace(base_trace, &mut combined_resource_spans, &mut failed);

    for trace in traces_iter {
        all_invocation_ids.extend(trace.invocation_ids.clone());
        process_trace(&trace, &mut combined_resource_spans, &mut failed);
    }

    (combined_resource_spans, all_invocation_ids, failed)
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

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 2);
        assert_eq!(invocation_ids, vec!["inv-1".to_string()]);
        assert!(failed.is_empty());
    }

    #[test]
    fn test_combine_traces_multiple_valid_traces() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 2);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 3);
        let trace3 = create_valid_trace(vec!["inv-3".to_string()], 1);
        let traces_iter = vec![trace2, trace3].into_iter();

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 6); // 2 + 3 + 1
        assert_eq!(
            invocation_ids,
            vec![
                "inv-1".to_string(),
                "inv-2".to_string(),
                "inv-3".to_string()
            ]
        );
        assert!(failed.is_empty());
    }

    #[test]
    fn test_combine_traces_with_invalid_base_trace() {
        let base_trace = create_invalid_trace(vec!["inv-1".to_string()]);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 2);
        let traces_iter = vec![trace2].into_iter();

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 2); // Only from trace2
        assert_eq!(
            invocation_ids,
            vec!["inv-1".to_string(), "inv-2".to_string()]
        );
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].invocation_ids, vec!["inv-1".to_string()]);
    }

    #[test]
    fn test_combine_traces_with_invalid_subsequent_trace() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 2);
        let trace2 = create_invalid_trace(vec!["inv-2".to_string()]);
        let trace3 = create_valid_trace(vec!["inv-3".to_string()], 1);
        let traces_iter = vec![trace2.clone(), trace3].into_iter();

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 3); // 2 from base + 1 from trace3
        assert_eq!(
            invocation_ids,
            vec![
                "inv-1".to_string(),
                "inv-2".to_string(),
                "inv-3".to_string()
            ]
        );
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].invocation_ids, vec!["inv-2".to_string()]);
    }

    #[test]
    fn test_combine_traces_all_invalid() {
        let base_trace = create_invalid_trace(vec!["inv-1".to_string()]);
        let trace2 = create_invalid_trace(vec!["inv-2".to_string()]);
        let traces_iter = vec![trace2].into_iter();

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert!(resource_spans.is_empty());
        assert_eq!(
            invocation_ids,
            vec!["inv-1".to_string(), "inv-2".to_string()]
        );
        assert_eq!(failed.len(), 2);
    }

    #[test]
    fn test_combine_traces_with_multiple_invocation_ids() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string(), "inv-2".to_string()], 1);
        let trace2 = create_valid_trace(vec!["inv-3".to_string(), "inv-4".to_string()], 1);
        let traces_iter = vec![trace2].into_iter();

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert_eq!(resource_spans.len(), 2);
        assert_eq!(
            invocation_ids,
            vec![
                "inv-1".to_string(),
                "inv-2".to_string(),
                "inv-3".to_string(),
                "inv-4".to_string()
            ]
        );
        assert!(failed.is_empty());
    }

    #[test]
    fn test_combine_traces_empty_resource_spans() {
        let base_trace = create_valid_trace(vec!["inv-1".to_string()], 0);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 0);
        let traces_iter = vec![trace2].into_iter();

        let (resource_spans, invocation_ids, failed) = combine_traces(&base_trace, traces_iter);

        assert!(resource_spans.is_empty());
        assert_eq!(
            invocation_ids,
            vec!["inv-1".to_string(), "inv-2".to_string()]
        );
        assert!(failed.is_empty());
    }

    #[test]
    fn test_build_traces_request_happy_flow() {
        use std::env;

        // Set up environment variable for endpoint
        env::set_var("x_LUMIGO_ENDPOINT", "https://example.com:443/v1/traces");

        // Create valid traces
        let trace1 = create_valid_trace(vec!["inv-1".to_string()], 2);
        let trace2 = create_valid_trace(vec!["inv-2".to_string()], 3);
        let traces = vec![trace1.clone(), trace2];
        let original_traces = traces.clone();

        // Call _build_traces_request
        let result = _build_traces_request(traces, &original_traces);

        // Verify we got a result
        assert!(result.is_some());
        let (req, combined_trace, failed) = result.unwrap();

        // Verify the request was built
        assert_eq!(req.method(), &Method::POST);
        assert_eq!(req.uri().scheme_str(), Some("https"));
        assert_eq!(req.uri().authority().unwrap().as_str(), "example.com:443");
        assert_eq!(req.uri().path(), "/v1/traces");

        // Verify headers
        let headers = req.headers();
        assert!(headers.contains_key(header::HOST));
        assert!(headers.contains_key(header::CONTENT_LENGTH));

        // Verify combined trace
        assert_eq!(combined_trace.invocation_ids.len(), 2);
        assert_eq!(combined_trace.invocation_ids[0], "inv-1");
        assert_eq!(combined_trace.invocation_ids[1], "inv-2");

        // Verify the body is valid protobuf
        let decoded = ExportTraceServiceRequest::decode(combined_trace.body.as_slice()).unwrap();
        assert_eq!(decoded.resource_spans.len(), 5); // 2 + 3

        // Verify no failures
        assert!(failed.is_empty());

        // Clean up
        env::remove_var("x_LUMIGO_ENDPOINT");
    }
}

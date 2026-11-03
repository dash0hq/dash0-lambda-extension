use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use once_cell::sync::Lazy;
use std::time::{Duration, Instant};

use crate::route::ReqBody;
use crate::state;
use crate::util::parsers::{generate_random_span_id, get_span_id_from_invocation_id};

static HTTP_CLIENT: Lazy<Client<hyper_util::client::legacy::connect::HttpConnector, ReqBody>> =
    Lazy::new(|| Client::builder(TokioExecutor::new()).build_http());

const EXTENSION_API_VERSION: &str = "2020-01-01";

fn make_uri(path: &str) -> hyper::Uri {
    match hyper::Uri::builder()
        .scheme("http")
        .authority(crate::config::endpoints::sandbox_runtime_api())
        .path_and_query(format!("/{}/extension{}", EXTENSION_API_VERSION, path))
        .build()
    {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!(
                "[{}] Error building Lambda Extensions API endpoint URL: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            panic!(
                "[{}] Failed to build Extensions API URI - severe misconfiguration: {}",
                crate::log_prefix_with("Extension"),
                e
            );
        }
    }
}

fn handle_invoke_event(json: &serde_json::Value) {
    if let Some(arn) = json.get("invokedFunctionArn").and_then(|v| v.as_str()) {
        state::global::store_function_arn(arn);
    }

    // Parse trace context from _X_AMZN_TRACE_ID tracing header
    if let Some(request_id) = json.get("requestId").and_then(|v| v.as_str()) {
        if let Some(trace_value) = json
            .get("tracing")
            .and_then(|t| t.get("value"))
            .and_then(|v| v.as_str())
        {
            let parsed = crate::otlp::span_link_extractor::parse_amzn_trace_id(trace_value);
            // Store the raw header even when it cannot be parsed, so the root span still
            // carries it for debugging.
            state::invocation_entry::update(request_id, |entry| {
                entry.x_amzn_trace_id = Some(trace_value.to_string());
                if let Some((trace_id_bytes, parent_span_id_bytes, sampled)) = parsed {
                    let parent_span_id = hex::encode(&parent_span_id_bytes);
                    entry.trace_id = Some(hex::encode(&trace_id_bytes));
                    entry.span_id.get_or_insert_with(|| {
                        hex::encode(get_span_id_from_invocation_id(request_id))
                    });
                    entry
                        .root_span_id
                        .get_or_insert_with(|| hex::encode(generate_random_span_id()));
                    entry.parent_span_id = Some(
                        if !sampled && !crate::config::user::is_xray_traces_enabled() {
                            String::new()
                        } else {
                            parent_span_id
                        },
                    );
                    entry.sampled = sampled;
                }
            });
            tracing::info!(
                "[{}] Parsed trace context from _X_AMZN_TRACE_ID: {}",
                crate::log_prefix_with("Extension"),
                trace_value
            );
        }
    }
}

async fn flush_if_needed(
    event_type: Option<&str>,
    shutdown_reason: Option<&str>,
    invocation_id: Option<&str>,
) {
    let should_flush = (matches!(event_type, Some("INVOKE"))
        && !crate::config::is_send_on_invocation_end())
        || (matches!(event_type, Some("SHUTDOWN")) && shutdown_reason == Some("spindown"));

    if !should_flush {
        return;
    }
    let is_invocation_end = matches!(event_type, Some("SHUTDOWN"));
    if is_invocation_end {
        // on shutdown/spindown wait for logs to arrive
        tokio::time::sleep(Duration::from_millis(200)).await;
    } else {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    crate::otlp::exporter::flush_traces().await;
    crate::otlp::exporter::flush_logs(invocation_id).await;
    crate::state::invocation_entry::delete_done_invocations();
}

async fn wait_for_runtime_done() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::state::invocation_data::store_runtime_done_notifier(tx);

    tracing::info!(
        "[{}] Waiting for platform.runtimeDone",
        crate::log_prefix_with("Extension")
    );

    // Wait for the signal with a timeout to prevent indefinite blocking
    match tokio::time::timeout(
        std::time::Duration::from_secs(900), // 15 minute timeout (max Lambda duration)
        rx,
    )
    .await
    {
        Ok(Ok(())) => {
            tracing::info!(
                "[{}] Received platform.runtimeDone signal",
                crate::log_prefix_with("Extension")
            );
            crate::otlp::exporter::flush_traces().await;
            crate::otlp::exporter::flush_logs(None).await;
            crate::state::invocation_entry::delete_done_invocations();
        }
        Ok(Err(_)) => {
            tracing::warn!(
                "[{}] platform.runtimeDone channel closed",
                crate::log_prefix_with("Extension")
            );
        }
        Err(_) => {
            tracing::error!(
                "[{}] Timeout waiting for platform.runtimeDone",
                crate::log_prefix_with("Extension")
            );
        }
    }
}

/// Get next event from the Lambda Extensions API
///
pub async fn get_next() {
    let uri = make_uri("/event/next");

    let mut request = match hyper::Request::builder()
        .method("GET")
        .uri(uri)
        .body(Full::new(Bytes::new()))
    {
        Ok(req) => req,
        Err(e) => {
            tracing::error!(
                "[{}] Cannot create Lambda Extensions API request for get_next: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            return;
        }
    };

    match crate::extension::register::extension_id().try_into() {
        Ok(header_value) => {
            request
                .headers_mut()
                .insert("Lambda-Extension-Identifier", header_value);
        }
        Err(e) => {
            tracing::error!(
                "[{}] Invalid extension identifier for get_next: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            return;
        }
    }

    let start = Instant::now();
    match HTTP_CLIENT.request(request).await {
        Ok(response) => {
            let status = response.status();
            let body_bytes = match response.into_body().collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(err) => {
                    tracing::error!(
                        "[{}] Failed to read extension event body: {}",
                        crate::log_prefix_with("Extension"),
                        err
                    );
                    return;
                }
            };

            tracing::info!(
                "[{}] Event status={} payload={} latency={} ms",
                crate::log_prefix_with("Extension"),
                status,
                String::from_utf8_lossy(&body_bytes),
                start.elapsed().as_millis()
            );

            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                let event_type = json.get("eventType").and_then(|v| v.as_str());
                let is_invoke = matches!(event_type, Some("INVOKE"));

                if is_invoke {
                    handle_invoke_event(&json);
                    crate::otlp::flusher::invocation_started();
                }

                let invocation_id = json.get("requestId").and_then(|v| v.as_str());
                let shutdown_reason = json
                    .get("shutdownReason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase());
                flush_if_needed(event_type, shutdown_reason.as_deref(), invocation_id).await;

                if is_invoke && crate::config::is_send_on_invocation_end() {
                    wait_for_runtime_done().await;
                }
            }
        }
        Err(err) => {
            tracing::error!(
                "[{}] Error fetching next extension event: {}",
                crate::log_prefix_with("Extension"),
                err
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn invoke_event(request_id: &str, tracing_value: &str) -> serde_json::Value {
        serde_json::json!({
            "eventType": "INVOKE",
            "requestId": request_id,
            "tracing": { "value": tracing_value },
        })
    }

    #[test]
    #[serial]
    fn handle_invoke_event_parses_valid_trace_context_and_stores_raw_header() {
        let request_id = "events-test-valid";
        state::invocation_entry::remove(request_id);

        let raw = "Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1";
        handle_invoke_event(&invoke_event(request_id, raw));

        let entry = state::invocation_entry::get(request_id).expect("entry should be created");
        state::invocation_entry::remove(request_id);

        assert_eq!(entry.x_amzn_trace_id.as_deref(), Some(raw));
        assert_eq!(
            entry.trace_id.as_deref(),
            Some("698f814c7708a2b018bc2cc4726a6288")
        );
        assert_eq!(entry.parent_span_id.as_deref(), Some("f21a582b8b8134b9"));
        assert!(entry.span_id.is_some());
        assert!(entry.root_span_id.is_some());
        assert!(entry.sampled);
    }

    #[test]
    #[serial]
    fn handle_invoke_event_stores_raw_header_even_when_unparseable() {
        let request_id = "events-test-unparseable";
        state::invocation_entry::remove(request_id);

        let raw = "not-a-valid-x-amzn-trace-id-header";
        handle_invoke_event(&invoke_event(request_id, raw));

        let entry = state::invocation_entry::get(request_id).expect("entry should be created");
        state::invocation_entry::remove(request_id);

        assert_eq!(entry.x_amzn_trace_id.as_deref(), Some(raw));
        assert_eq!(entry.trace_id, None);
        assert_eq!(entry.span_id, None);
        assert_eq!(entry.root_span_id, None);
        assert_eq!(entry.parent_span_id, None);
        assert!(!entry.sampled);
    }

    // ── IDs of logs sent by an early flush ───────────────────────────
    //
    // The INVOKE event carries trace X and the Lambda service segment; the
    // runtime's `Lambda-Runtime-Trace-Id` carries X and the function segment
    // FS, the parent of the handler span in X-Ray mode. The handler continues
    // either the X-Ray context or an upstream traceparent with trace U and
    // caller PU.

    const X_HEADER: &str = "Root=1-11111111-111111111111111111111111;Parent=2222222222222222";
    const X: &str = "11111111111111111111111111111111";
    const FS: &str = "3333333333333333";
    const X_BYTES: [u8; 16] = [0x11; 16];
    const FS_BYTES: [u8; 8] = [0x33; 8];
    const U: [u8; 16] = [0xAA; 16];
    const PU: [u8; 8] = [0xBB; 8];
    const H: [u8; 8] = [0xCC; 8];

    fn set_mode(xray: bool, auto_instrumented: bool) {
        if xray {
            std::env::set_var("DASH0_XRAY_TRACES_ENABLED", "true");
        } else {
            std::env::remove_var("DASH0_XRAY_TRACES_ENABLED");
        }
        if auto_instrumented {
            std::env::set_var("AWS_LAMBDA_EXEC_WRAPPER", "/opt/wrapper");
        } else {
            std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
        }
        std::env::remove_var("DASH0_DISABLE_AUTO_INSTRUMENTATION");
    }

    fn reset_mode() {
        std::env::remove_var("DASH0_XRAY_TRACES_ENABLED");
        std::env::remove_var("AWS_LAMBDA_EXEC_WRAPPER");
    }

    fn invoke(request_id: &str, sampled: bool) {
        state::invocation_entry::remove(request_id);
        let header = format!("{X_HEADER};Sampled={}", u8::from(sampled));
        handle_invoke_event(&invoke_event(request_id, &header));
        let runtime_header = format!(
            "Root=1-11111111-111111111111111111111111;Parent={FS};Sampled={}",
            u8::from(sampled)
        );
        state::invocation_entry::record_runtime_trace_id(request_id, &runtime_header);
    }

    fn function_log(request_id: &str) {
        state::invocation_entry::store_telemetry_logs(vec![state::invocation_data::TelemetryLog {
            time: "2026-11-02T10:00:00Z".to_string(),
            r#type: "function".to_string(),
            record: serde_json::Value::String("hello".to_string()),
            invocation_id: Some(request_id.to_string()),
            trace_id: None,
            span_id: None,
            custom_attributes: vec![],
        }]);
    }

    /// Runs the early flush's log selection and returns the (trace ID, span
    /// ID) of each log of `request_id` it would send.
    fn early_flush(request_id: &str, max_held_bytes: usize) -> Vec<(String, String)> {
        let logs = state::invocation_entry::take_telemetry_logs_while_running(max_held_bytes);
        let (mine, others): (Vec<_>, Vec<_>) = logs
            .into_iter()
            .partition(|l| l.invocation_id.as_deref() == Some(request_id));
        drop(others);
        crate::otlp::log_mutations::map_logs_to_otlp(&mine)
            .iter()
            .map(|r| (hex::encode(&r.trace_id), hex::encode(&r.span_id)))
            .collect()
    }

    fn handler_span_arrives(request_id: &str, trace_id: [u8; 16], parent: [u8; 8]) {
        use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
        use opentelemetry_proto::tonic::common::v1::{
            any_value, AnyValue, InstrumentationScope, KeyValue,
        };
        use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
        let mut request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "@opentelemetry/instrumentation-aws-lambda".to_string(),
                        ..Default::default()
                    }),
                    spans: vec![Span {
                        trace_id: trace_id.to_vec(),
                        span_id: H.to_vec(),
                        parent_span_id: parent.to_vec(),
                        attributes: vec![KeyValue {
                            key: "faas.invocation_id".to_string(),
                            value: Some(AnyValue {
                                value: Some(any_value::Value::StringValue(request_id.to_string())),
                            }),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut ids = Vec::new();
        let mut body = Vec::new();
        crate::otlp::span_mutations::process_trace_request(&mut request, &mut ids, &mut body);
        assert_eq!(ids, vec![request_id.to_string()]);
    }

    fn root_span_id(request_id: &str) -> String {
        state::invocation_entry::get_root_span_id(request_id).expect("root span id")
    }

    #[test]
    #[serial]
    fn default_mode_holds_logs_until_a_child_span_tells_the_upstream_trace() {
        set_mode(false, true);
        let id = "early-default-upstream";
        invoke(id, true);
        let r = root_span_id(id);

        function_log(id);
        assert!(
            early_flush(id, usize::MAX).is_empty(),
            "the trace may still change: hold the log"
        );

        // A child span shows the handler continues the upstream trace U.
        state::invocation_entry::record_child_trace_id(id, &U);
        assert_eq!(
            early_flush(id, usize::MAX),
            vec![(hex::encode(U), r.clone())]
        );

        // The handler span confirms it; later logs keep the same IDs, and the
        // root span still hangs under the upstream caller.
        handler_span_arrives(id, U, PU);
        function_log(id);
        assert_eq!(early_flush(id, usize::MAX), vec![(hex::encode(U), r)]);
        let entry = state::invocation_entry::remove(id).unwrap();
        assert_eq!(
            entry.parent_span_id.as_deref(),
            Some(hex::encode(PU).as_str())
        );
        reset_mode();
    }

    #[test]
    #[serial]
    fn default_mode_without_auto_instrumentation_sends_logs_at_once() {
        set_mode(false, false);
        let id = "early-default-uninstrumented";
        invoke(id, true);
        let r = root_span_id(id);

        // No spans will arrive; the root span uses the X-Ray trace.
        function_log(id);
        assert_eq!(early_flush(id, usize::MAX), vec![(X.to_string(), r)]);
        state::invocation_entry::remove(id);
        reset_mode();
    }

    #[test]
    #[serial]
    fn xray_mode_links_logs_to_the_function_segment() {
        set_mode(true, true);
        let id = "early-xray-sampled";
        invoke(id, true);

        function_log(id);
        assert_eq!(
            early_flush(id, usize::MAX),
            vec![(X.to_string(), FS.to_string())]
        );

        // Still the segment once the handler span is known, so all logs of
        // the invocation sit on the same span.
        handler_span_arrives(id, X_BYTES, FS_BYTES);
        function_log(id);
        assert_eq!(
            early_flush(id, usize::MAX),
            vec![(X.to_string(), FS.to_string())]
        );
        state::invocation_entry::remove(id);
        reset_mode();
    }

    #[test]
    #[serial]
    fn xray_mode_unsampled_holds_logs_until_the_handler_span() {
        set_mode(true, true);
        let id = "early-xray-unsampled";
        invoke(id, false);

        // Unsampled: AWS records no segment, so logs need the handler span.
        function_log(id);
        assert!(early_flush(id, usize::MAX).is_empty());

        handler_span_arrives(id, U, PU);
        assert_eq!(
            early_flush(id, usize::MAX),
            vec![(hex::encode(U), hex::encode(H))]
        );
        state::invocation_entry::remove(id);
        reset_mode();
    }

    #[test]
    #[serial]
    fn held_logs_are_sent_once_they_reach_the_limit() {
        set_mode(false, true);
        let id = "early-default-overflow";
        invoke(id, true);
        let r = root_span_id(id);

        function_log(id);
        let held = state::invocation_entry::get(id).unwrap().logs_bytes;
        assert!(
            early_flush(id, held + 1).is_empty(),
            "below the limit: held"
        );
        assert_eq!(
            early_flush(id, held),
            vec![(X.to_string(), r)],
            "at the limit: sent with the IDs known so far"
        );
        state::invocation_entry::remove(id);
        reset_mode();
    }
}

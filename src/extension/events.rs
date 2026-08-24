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
}

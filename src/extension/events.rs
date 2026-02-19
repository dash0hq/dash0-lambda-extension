use hyper::Body;
use std::time::{Duration, Instant};

use crate::state;

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

/// Get next event from the Lambda Extensions API
///
pub async fn get_next() {
    let uri = make_uri("/event/next");

    let mut request = match hyper::Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
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
    match hyper::Client::new().request(request).await {
        Ok(response) => {
            let status = response.status();
            let body_bytes = match hyper::body::to_bytes(response.into_body()).await {
                Ok(bytes) => bytes,
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
                if json
                    .get("eventType")
                    .and_then(|v| v.as_str())
                    .map(|t| t == "INVOKE")
                    == Some(true)
                {
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
                            if let Some((trace_id_bytes, parent_span_id_bytes)) =
                                crate::otlp::span_link_extractor::parse_amzn_trace_id(trace_value)
                            {
                                let trace_id = hex::encode(&trace_id_bytes);
                                let parent_span_id = hex::encode(&parent_span_id_bytes);
                                state::invocation_data::store_invocation_span_id(
                                    request_id,
                                    trace_id,
                                    String::new(),
                                    parent_span_id,
                                );
                            }
                        }
                    }
                }

                let event_type = json.get("eventType").and_then(|v| v.as_str());
                let shutdown_reason = json
                    .get("shutdownReason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase());

                let should_flush = (matches!(event_type, Some("INVOKE"))
                    && !crate::config::is_send_on_invocation_end())
                    || (matches!(event_type, Some("SHUTDOWN"))
                        && shutdown_reason.as_deref() == Some("spindown"));

                if should_flush {
                    let is_invocation_end = matches!(event_type, Some("SHUTDOWN"));
                    if is_invocation_end {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        crate::otlp::exporter::flush_traces().await;
                    } else {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    crate::otlp::exporter::flush_logs(is_invocation_end).await;
                }

                if matches!(event_type, Some("INVOKE"))
                    && crate::config::is_send_on_invocation_end()
                {
                    // Block execution until platform.runtimeDone is received
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
                            crate::otlp::exporter::flush_logs(true).await;
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

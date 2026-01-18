use hyper::{Body, Error, Request, Response};

use crate::otlp::exporter::{flush_logs, flush_traces, send_traces};
use crate::otlp::span_mutations::build_runtime_error_trace;
use crate::state;
use crate::state::invocation_data::take_traces;
use crate::util::parsers::extract_error_invocation_ids;

pub async fn telemetry(req: Request<Body>) -> Result<Response<Body>, Error> {
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body_bytes);

    let error_invocation_ids = extract_error_invocation_ids(&body_bytes, body_text.as_ref());

    tracing::info!(
        "[{}] telemetry event path={} len={} body={}",
        crate::log_prefix(),
        parts.uri.path(),
        body_bytes.len(),
        body_text
    );

    if let Ok(mut logs) =
        serde_json::from_str::<Vec<crate::state::invocation_data::TelemetryLog>>(&body_text)
    {
        crate::util::log_processing::process_telemetry_logs(&mut logs);

        let mut report_invocation_ids: Vec<String> = Vec::new();
        for log in &logs {
            if log.r#type == "platform.report" {
                if let Some(id) = &log.invocation_id {
                    report_invocation_ids.push(id.clone());
                }
            }
        }
        crate::state::invocation_data::store_telemetry_logs(logs);

        if !report_invocation_ids.is_empty() {
            if !crate::config::is_send_on_invocation_end() {
                flush_traces().await;
            }
            for id in &report_invocation_ids {
                crate::state::invocation_data::cleanup_invocation(id);
            }
        }
    } else {
        tracing::debug!(
            "[{}] Failed to deserialize telemetry logs from body",
            crate::log_prefix()
        );
    }

    if !error_invocation_ids.is_empty() {
        tracing::info!(
            "[{}] telemetry runtimeDone error detected for invocations: {:?} body={}",
            crate::log_prefix(),
            error_invocation_ids,
            body_text
        );

        // Fetch account ID if not already cached
        if state::global::get_account_id()
            .map(|id| id.is_empty())
            .unwrap_or(true)
        {
            let _ = tokio::task::spawn(async { state::global::fetch_and_cache_account_id().await })
                .await;
        }

        let mut traces_to_send = take_traces();

        for (invocation_id, error_type) in &error_invocation_ids {
            match build_runtime_error_trace(invocation_id, Some(error_type), None, &traces_to_send)
            {
                Some(trace) => traces_to_send.push(trace),
                None => {
                    tracing::error!(
                        "[{}] Failed to build runtimeDone trace for invocation {}",
                        crate::log_prefix(),
                        invocation_id
                    );
                }
            }
        }

        if !traces_to_send.is_empty() {
            send_traces(traces_to_send).await;
        }
        flush_logs(true).await;
    }

    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

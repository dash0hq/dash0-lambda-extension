use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response};

use crate::config::user::is_telemetry_log_collection_disabled;
use crate::otlp::exporter::{flush_telemetry_logs, send_traces};
use crate::otlp::metrics_creation::create_supplementary_metrics;
use crate::otlp::span_creation::{create_overhead_supplementary_span, create_supplementary_spans};
use crate::otlp::span_mutations::build_synthetic_trace;
use crate::route::{empty_body, ResBody};
use crate::state::invocation_entry;
use crate::util::parsers::extract_error_invocation_ids;

pub async fn telemetry(req: Request<Incoming>) -> Result<Response<ResBody>, hyper::Error> {
    let (parts, body) = req.into_parts();
    let body_bytes = body
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body_bytes);

    let error_invocation_ids = extract_error_invocation_ids(&body_bytes, body_text.as_ref());

    tracing::info!(
        "[{}] telemetry event path={} len={} body={}",
        crate::log_prefix(),
        parts.uri.path(),
        body_bytes.len(),
        body_text
    );

    let mut saw_runtime_done = false;

    if let Ok(mut logs) =
        serde_json::from_str::<Vec<crate::state::invocation_data::TelemetryLog>>(&body_text)
    {
        crate::util::log_processing::process_telemetry_logs(&mut logs);

        for log in &logs {
            if log.r#type == "platform.runtimeDone" {
                saw_runtime_done = true;
                if let Some(id) = &log.invocation_id {
                    let is_error = error_invocation_ids.iter().any(|(eid, _)| eid == id);
                    if !is_error {
                        create_supplementary_spans(id, true);
                    }
                }
            }

            if log.r#type == "platform.report" {
                if let Some(id) = &log.invocation_id {
                    create_overhead_supplementary_span(id);
                    create_supplementary_metrics(id);
                    invocation_entry::update(id, |entry| {
                        entry.state = crate::state::invocation_entry::InvocationState::Done;
                        entry.init_duration = 0.0;
                    });
                }
            }
        }

        if !is_telemetry_log_collection_disabled() {
            crate::state::invocation_entry::store_telemetry_logs(logs);
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

        let mut traces_to_send = invocation_entry::take_all_traces();

        for (invocation_id, error_type) in &error_invocation_ids {
            match build_synthetic_trace(invocation_id, Some(error_type), None, &traces_to_send) {
                Some(trace) => {
                    traces_to_send.push(trace);
                    if let Some(supp) = create_supplementary_spans(invocation_id, false) {
                        traces_to_send.push(supp);
                    }
                }
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
        flush_telemetry_logs(None).await;
    }

    // Signal platform.runtimeDone only after error traces have been built and
    // sent. Signaling earlier lets the send-on-invocation-end flush task drain
    // the stored traces concurrently, so build_synthetic_trace can no longer
    // recover the invocation's real trace id and correlation is lost.
    if saw_runtime_done {
        if let Some(notifier) = crate::state::invocation_data::take_runtime_done_notifier() {
            tracing::info!("[{}] Signaled platform.runtimeDone", crate::log_prefix());
            let _ = notifier.send(());
        }
    }

    Ok(Response::builder().status(200).body(empty_body()).unwrap())
}

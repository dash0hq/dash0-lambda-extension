use crate::config::user::is_logs_instrumentation_enabled;
use crate::otlp::exporter::{flush_telemetry_logs, send_traces};
use crate::otlp::metrics_creation::create_supplementary_metrics;
use crate::otlp::span_creation::{create_overhead_supplementary_span, create_supplementary_spans};
use crate::otlp::span_mutations::build_synthetic_trace;
use crate::state::invocation_entry;
use crate::util::parsers::extract_error_invocation_ids;
use hyper::{Body, Error, Request, Response};

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

        for log in &logs {
            if log.r#type == "platform.runtimeDone" {
                if let Some(id) = &log.invocation_id {
                    create_supplementary_spans(id);
                }
                if let Some(notifier) = crate::state::invocation_data::take_runtime_done_notifier()
                {
                    tracing::info!("[{}] Signaled platform.runtimeDone", crate::log_prefix());
                    let _ = notifier.send(());
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

        if !is_logs_instrumentation_enabled() {
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
        flush_telemetry_logs(None).await;
    }

    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

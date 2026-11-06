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

    if let Ok(mut logs) =
        serde_json::from_str::<Vec<crate::state::invocation_data::TelemetryLog>>(&body_text)
    {
        crate::util::log_processing::process_telemetry_logs(&mut logs);

        for log in &logs {
            if log.r#type == "platform.runtimeDone" {
                crate::otlp::flusher::invocation_finished();
                if let Some(id) = &log.invocation_id {
                    let is_error = error_invocation_ids.iter().any(|(eid, _)| eid == id);
                    if !is_error {
                        create_supplementary_spans(id, true);
                    }
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
                    });
                }
            }
        }

        if !is_telemetry_log_collection_disabled() {
            if !crate::otlp::flusher::admit(body_bytes.len(), "function logs") {
                // Buffer full: keep only the few platform records.
                logs.retain(|log| log.r#type.starts_with("platform."));
            }
            crate::state::invocation_entry::store_telemetry_logs(logs);
            crate::otlp::flusher::stored();
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

        tokio::join!(send_traces(traces_to_send), flush_telemetry_logs(None));
    }

    Ok(Response::builder().status(200).body(empty_body()).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use prost::Message;
    use serial_test::serial;
    use tokio::net::TcpListener;

    async fn post_telemetry(body: serde_json::Value) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service_fn(telemetry))
                .await
                .unwrap();
        });
        let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
        let request = Request::post(format!("http://{address}/2018-06-01/telemetry"))
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap();
        let response = client.request(request).await.unwrap();
        assert_eq!(response.status(), 200);
        response.into_body().collect().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    #[serial]
    async fn report_preserves_cold_start_data_for_supplementary_spans() {
        for status in ["success", "error", "timeout"] {
            for init_duration in [482.11, 0.0] {
                let id = format!("report-init-{status}-{init_duration}");
                invocation_entry::update(&id, |entry| {
                    entry.trace_id = Some("aa".repeat(16));
                    entry.root_span_id = Some("bb".repeat(8));
                    entry.span_id = Some("cc".repeat(8));
                });
                let body = serde_json::json!([
                    {
                        "time": "2026-11-03T16:01:18.000Z",
                        "type": "platform.start",
                        "record": { "requestId": id }
                    },
                    {
                        "time": "2026-11-03T16:01:28.000Z",
                        "type": "platform.runtimeDone",
                        "record": {
                            "requestId": id, "status": status,
                            "metrics": { "durationMs": 10000.0 }
                        }
                    },
                    {
                        "time": "2026-11-03T16:01:28.001Z",
                        "type": "platform.report",
                        "record": {
                            "requestId": id,
                            "metrics": {
                                "durationMs": 10000.0, "billedDurationMs": 10483.0,
                                "initDurationMs": init_duration, "maxMemoryUsedMB": 115
                            }
                        }
                    }
                ]);
                post_telemetry(body).await;

                let trace = crate::otlp::span_creation::create_spans(&id, true, false).unwrap();
                let export = ExportTraceServiceRequest::decode(trace.body.as_slice()).unwrap();
                let spans = &export.resource_spans[0].scope_spans[0].spans;
                assert_eq!(
                    spans
                        .iter()
                        .any(|span| span.name == "aws.lambda.initialization"),
                    init_duration > 0.0,
                    "status={status}, init_duration={init_duration}"
                );
                invocation_entry::remove(&id);
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn init_report_before_invocation_is_attached_to_first_start_only() {
        use opentelemetry_proto::tonic::common::v1::any_value::Value;

        for batched in [false, true] {
            // Runtime /next and telemetry are independent HTTP streams.
            crate::state::invocation_data::store_current_invocation_id("stale-invocation");
            let init_report = serde_json::json!({
                "time": "2026-11-03T15:43:49.000Z", "type": "platform.initReport",
                "record": { "phase": "init", "metrics": { "durationMs": 1764.64 } }
            });
            if !batched {
                post_telemetry(serde_json::json!([init_report])).await;
            }
            for cold in [true, false] {
                let id = format!("init-before-start-{batched}-{cold}");
                invocation_entry::update(&id, |entry| {
                    entry.trace_id = Some("aa".repeat(16));
                    entry.root_span_id = Some("bb".repeat(8));
                    entry.span_id = Some("cc".repeat(8));
                });
                let mut logs = vec![];
                if batched && cold {
                    logs.push(init_report.clone());
                }
                logs.extend([
                    serde_json::json!({
                        "time": "2026-11-03T15:43:51.000Z", "type": "platform.start",
                        "record": { "requestId": id }
                    }),
                    serde_json::json!({
                        "time": "2026-11-03T15:43:51.254Z", "type": "platform.runtimeDone",
                        "record": { "requestId": id, "status": "success", "metrics": { "durationMs": 253.88 } }
                    }),
                ]);
                post_telemetry(serde_json::json!(logs)).await;
                let traces = invocation_entry::take_traces_by_id(&id);
                assert_eq!(traces.len(), 1);
                let export = ExportTraceServiceRequest::decode(traces[0].body.as_slice()).unwrap();
                let spans = &export.resource_spans[0].scope_spans[0].spans;
                assert_eq!(
                    spans.len(),
                    if cold { 2 } else { 1 },
                    "batched={batched}, cold={cold}"
                );
                let init = spans[0]
                    .attributes
                    .iter()
                    .find(|attr| attr.key == "faas.init_duration");
                if cold {
                    assert_eq!(
                        init.unwrap().value.as_ref().unwrap().value,
                        Some(Value::DoubleValue(1764.64))
                    );
                    assert_eq!(spans[1].name, "aws.lambda.initialization");
                    assert_eq!(spans[1].parent_span_id, spans[0].span_id);
                } else {
                    assert!(init.is_none());
                }
                // REPORT arrives later, and must not produce another root/init pair.
                post_telemetry(serde_json::json!([{
                    "time": "2026-11-03T15:43:51.255Z", "type": "platform.report",
                    "record": { "requestId": id, "metrics": { "durationMs": 253.88, "billedDurationMs": 2019.0, "initDurationMs": if cold { 1764.64 } else { 0.0 } } }
                }])).await;
                for trace in invocation_entry::take_traces_by_id(&id) {
                    let export = ExportTraceServiceRequest::decode(trace.body.as_slice()).unwrap();
                    assert!(export.resource_spans[0].scope_spans[0]
                        .spans
                        .iter()
                        .all(|span| span.name == "aws.lambda.overhead"));
                }
                invocation_entry::remove(&id);
            }
        }
        invocation_entry::remove("stale-invocation");
    }
}

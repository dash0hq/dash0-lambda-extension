use crate::store::{PayloadValue, TelemetryLog};

pub async fn process_telemetry_logs(logs: &mut Vec<TelemetryLog>) {
    let mut current_invocation_id = crate::store::get_last_seen_invocation_start().await;

    for log in logs {
        if log.r#type == "platform.start" {
            parse_platform_start(log, &mut current_invocation_id).await;
        }

        if log.r#type == "platform.initReport" {
            parse_platform_init_report(log).await;
        }

        if log.r#type == "platform.runtimeDone" {
            parse_platform_runtime_done(log).await;
        }

        if log.r#type == "platform.report" {
            parse_platform_report(log).await;
        }

        // For platform logs, extract invocation ID from the log record itself (safer than state)
        // For other logs, use the current invocation ID from state
        let invocation_id = if log.r#type == "platform.start"
            || log.r#type == "platform.runtimeDone"
            || log.r#type == "platform.report"
        {
            log.record
                .as_object()
                .and_then(|record| record.get("requestId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| current_invocation_id.as_ref().map(|v| v.to_string()))
        } else {
            current_invocation_id.as_ref().map(|v| v.to_string())
        };

        log.invocation_id = invocation_id;
    }
}

async fn parse_platform_start(log: &TelemetryLog, current_invocation_id: &mut Option<PayloadValue>) {
    if let Some(record) = log.record.as_object() {
        if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
            crate::store::store_last_seen_invocation_start(req_id).await;
            // Update the current invocation ID from store to get the right type (String or Arc)
            *current_invocation_id = crate::store::get_last_seen_invocation_start().await;

            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&log.time) {
                let start_time = dt.timestamp_millis() as f64;
                crate::store::update_invocation_data(req_id, |data| {
                    data.start_time = start_time;
                }).await;
            } else {
                tracing::info!(
                    "[{}] Failed to parse platform.start log time: {}",
                    crate::log_prefix(),
                    log.time
                );
            }
        }
    }
}

async fn parse_platform_init_report(log: &TelemetryLog) {
    if let Some(duration_ms) = log
        .record
        .get("metrics")
        .and_then(|m| m.get("durationMs"))
        .and_then(|d| d.as_f64())
    {
        if let Some(req_id) = crate::store::get_current_invocation_id().await {
            crate::store::update_invocation_data(req_id.as_str(), |data| {
                data.init_duration = duration_ms;
            }).await;
        }
    }
}

async fn parse_platform_runtime_done(log: &TelemetryLog) {
    if let Some(record) = log.record.as_object() {
        if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
            let end_time = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&log.time) {
                dt.timestamp_millis() as f64
            } else {
                tracing::info!(
                    "[{}] Failed to parse platform.runtimeDone log time: {}",
                    crate::log_prefix(),
                    log.time
                );
                0.0
            };

            let duration = record
                .get("metrics")
                .and_then(|m| m.get("durationMs"))
                .and_then(|d| d.as_f64())
                .unwrap_or(0.0);

            if end_time > 0.0 || duration > 0.0 {
                crate::store::update_invocation_data(req_id, |data| {
                    if end_time > 0.0 {
                        data.end_time = end_time;
                    }
                    if duration > 0.0 {
                        data.duration = duration;
                    }
                }).await;
            }
        }
    }
    if let Some(notifier) = crate::store::take_runtime_done_notifier().await {
        tracing::info!("[{}] Signaled platform.runtimeDone", crate::log_prefix());
        let _ = notifier.send(());
    }
}

async fn parse_platform_report(log: &TelemetryLog) {
    if let Some(record) = log.record.as_object() {
        if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
            let log_timestamp = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&log.time) {
                dt.timestamp_millis() as f64
            } else {
                tracing::info!(
                    "[{}] Failed to parse platform.report log time: {}",
                    crate::log_prefix(),
                    log.time
                );
                0.0
            };

            if let Some(metrics) = record.get("metrics").and_then(|m| m.as_object()) {
                let duration = metrics
                    .get("durationMs")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let billed_duration = metrics
                    .get("billedDurationMs")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let memory_usage = metrics
                    .get("maxMemoryUsedMB")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let init_duration = metrics
                    .get("initDurationMs")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                crate::store::update_invocation_data(req_id, |data| {
                    if data.start_time > 0.0 {
                        data.end_time = data.start_time + duration;
                    } else if log_timestamp > 0.0 {
                        data.end_time = log_timestamp;
                    }

                    data.duration = duration;
                    data.billed_duration = billed_duration;
                    data.memory_usage = memory_usage;
                    if init_duration > 0.0 {
                        data.init_duration = init_duration;
                    }
                }).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{get_invocation_data, TelemetryLog};
    use serde_json::json;
    use serial_test::serial;

    fn create_log(
        log_type: &str,
        time: &str,
        record: serde_json::Value,
        invocation_id: Option<String>,
    ) -> TelemetryLog {
        TelemetryLog {
            time: time.to_string(),
            r#type: log_type.to_string(),
            record,
            invocation_id,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_parse_platform_start() {
        let req_id = "test-req-start";
        let time_str = "2023-01-01T12:00:00.000Z";
        let logs = vec![create_log(
            "platform.start",
            time_str,
            json!({ "requestId": req_id }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs).await;

        let data = get_invocation_data(req_id).await.expect("Should have data");
        let expected_time = chrono::DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.start_time, expected_time);
    }

    #[tokio::test]
    #[serial]
    async fn test_parse_platform_init_report() {
        use crate::store::store_current_invocation_id;
        let req_id = "test-req-init";
        store_current_invocation_id(req_id).await; // Set context

        let logs = vec![create_log(
            "platform.initReport",
            "2023-01-01T12:00:00.000Z",
            json!({
                "metrics": { "durationMs": 123.45 }
            }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs).await;

        let data = get_invocation_data(req_id).await.expect("Should have data");
        assert_eq!(data.init_duration, 123.45);
    }

    #[tokio::test]
    #[serial]
    async fn test_parse_platform_runtime_done() {
        let req_id = "test-req-done";
        let time_str = "2023-01-01T12:00:01.000Z";
        let logs = vec![create_log(
            "platform.runtimeDone",
            time_str,
            json!({
                "requestId": req_id,
                "metrics": { "durationMs": 500.0 }
            }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs).await;

        let data = get_invocation_data(req_id).await.expect("Should have data");
        let expected_end = chrono::DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.end_time, expected_end);
        assert_eq!(data.duration, 500.0);
    }

    #[tokio::test]
    #[serial]
    async fn test_parse_platform_report() {
        let req_id = "test-req-report";
        let time_str = "2023-01-01T12:00:02.000Z";
        let logs = vec![create_log(
            "platform.report",
            time_str,
            json!({
                "requestId": req_id,
                "metrics": {
                    "durationMs": 600.0,
                    "billedDurationMs": 700.0,
                    "maxMemoryUsedMB": 128,
                    "initDurationMs": 50.0
                }
            }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs).await;

        let data = get_invocation_data(req_id).await.expect("Should have data");
        let expected_end = chrono::DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.end_time, expected_end);
        assert_eq!(data.duration, 600.0);
        assert_eq!(data.billed_duration, 700.0);
        assert_eq!(data.memory_usage, 128);
        assert_eq!(data.init_duration, 50.0);
    }

    #[tokio::test]
    #[serial]
    async fn test_process_telemetry_logs_full_flow() {
        let req_id = "test-full-flow";
        let start_time = "2023-01-01T12:00:00.000Z";
        let end_time = "2023-01-01T12:00:01.000Z";

        let mut logs = vec![
            create_log(
                "platform.start",
                start_time,
                json!({ "requestId": req_id }),
                None,
            ),
            create_log(
                "platform.runtimeDone",
                end_time,
                json!({
                    "requestId": req_id,
                    "metrics": { "durationMs": 1000.0 }
                }),
                None,
            ),
            // Report often sends updated metrics
            create_log(
                "platform.report",
                end_time,
                json!({
                    "requestId": req_id,
                    "metrics": {
                        "durationMs": 1000.1,
                        "billedDurationMs": 1001.0,
                        "maxMemoryUsedMB": 256,
                        "initDurationMs": 0.0 // Should not overwrite if 0? Actually current logic overwrites if > 0. Here it is 0.
                    }
                }),
                None,
            ),
        ];

        process_telemetry_logs(&mut logs).await;

        let data = get_invocation_data(req_id).await.expect("Should have data");
        let expected_start = chrono::DateTime::parse_from_rfc3339(start_time)
            .unwrap()
            .timestamp_millis() as f64;

        assert_eq!(data.start_time, expected_start);
        assert_eq!(data.end_time, expected_start + 1000.1);
        // Report duration should overwrite runtimeDone duration
        assert_eq!(data.duration, 1000.1);
        assert_eq!(data.billed_duration, 1001.0);
        assert_eq!(data.memory_usage, 256);
        // init_duration was not set in report (0.0), so check if it remains default (0.0) or if we had set it previously.
        // We didn't set it previously in this flow.
        assert_eq!(data.init_duration, 0.0);
    }
}

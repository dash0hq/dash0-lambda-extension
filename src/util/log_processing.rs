use crate::state::invocation_data::TelemetryLog;

pub fn process_telemetry_logs(logs: &mut Vec<TelemetryLog>) {
    let mut current_invocation_id = crate::state::invocation_data::get_last_seen_invocation_start();

    for log in logs {
        if log.r#type == "platform.start" {
            parse_platform_start(log, &mut current_invocation_id);
        }

        if log.r#type == "platform.initReport" {
            parse_platform_init_report(log);
        }

        if log.r#type == "platform.runtimeDone" {
            parse_platform_runtime_done(log);
        }

        if log.r#type == "platform.report" {
            parse_platform_report(log);
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
                .or_else(|| current_invocation_id.clone())
        } else {
            current_invocation_id.clone()
        };

        log.invocation_id = invocation_id;
    }
}

fn parse_platform_start(log: &TelemetryLog, current_invocation_id: &mut Option<String>) {
    if let Some(record) = log.record.as_object() {
        if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
            crate::state::invocation_data::store_last_seen_invocation_start(req_id);
            *current_invocation_id = Some(req_id.to_string());

            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&log.time) {
                let start_time = dt.timestamp_millis() as f64;
                crate::state::invocation_entry::update(req_id, |entry| {
                    entry.start_time = start_time;
                });
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

fn parse_platform_init_report(log: &TelemetryLog) {
    if let Some(duration_ms) = log
        .record
        .get("metrics")
        .and_then(|m| m.get("durationMs"))
        .and_then(|d| d.as_f64())
    {
        if let Some(req_id) = crate::state::invocation_data::get_current_invocation_id() {
            crate::state::invocation_entry::update(&req_id, |entry| {
                entry.init_duration = duration_ms;
            });
        }
    }
}

fn parse_platform_runtime_done(log: &TelemetryLog) {
    if let Some(record) = log.record.as_object() {
        if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
            // Try to get end_time from the responseDuration span's "start" field first,
            // falling back to the log's "time" field.
            let response_duration_start = record
                .get("spans")
                .and_then(|s| s.as_array())
                .and_then(|spans| {
                    spans.iter().find(|s| {
                        s.get("name").and_then(|n| n.as_str()) == Some("responseDuration")
                    })
                })
                .and_then(|s| s.get("start"))
                .and_then(|s| s.as_str());

            let time_str = response_duration_start.unwrap_or(&log.time);
            let end_time = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(time_str) {
                dt.timestamp_millis() as f64
            } else {
                tracing::info!(
                    "[{}] Failed to parse platform.runtimeDone end time: {}",
                    crate::log_prefix(),
                    time_str
                );
                0.0
            };

            let duration = record
                .get("metrics")
                .and_then(|m| m.get("durationMs"))
                .and_then(|d| d.as_f64())
                .unwrap_or(0.0);

            if end_time > 0.0 || duration > 0.0 {
                crate::state::invocation_entry::update(req_id, |entry| {
                    if end_time > 0.0 {
                        entry.end_time = end_time;
                    }
                    if duration > 0.0 {
                        entry.duration = duration;
                    }
                });
            }
        }
    }
}

fn parse_platform_report(log: &TelemetryLog) {
    if let Some(record) = log.record.as_object() {
        if let Some(req_id) = record.get("requestId").and_then(|v| v.as_str()) {
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

                crate::state::invocation_entry::update(req_id, |entry| {
                    entry.duration = duration;
                    entry.billed_duration = billed_duration;
                    entry.memory_usage = memory_usage;
                    if init_duration > 0.0 {
                        entry.init_duration = init_duration;
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::invocation_data::TelemetryLog;
    use crate::state::invocation_entry;
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
            trace_id: None,
            span_id: None,
        }
    }

    #[test]
    #[serial]
    fn test_parse_platform_start() {
        let req_id = "test-req-start";
        let time_str = "2023-01-01T12:00:00.000Z";
        let logs = vec![create_log(
            "platform.start",
            time_str,
            json!({ "requestId": req_id }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs);

        let data = invocation_entry::get(req_id).expect("Should have data");
        let expected_time = chrono::DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.start_time, expected_time);
    }

    #[test]
    #[serial]
    fn test_parse_platform_init_report() {
        use crate::state::invocation_data::store_current_invocation_id;
        let req_id = "test-req-init";
        store_current_invocation_id(req_id); // Set context

        let logs = vec![create_log(
            "platform.initReport",
            "2023-01-01T12:00:00.000Z",
            json!({
                "metrics": { "durationMs": 123.45 }
            }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs);

        let data = invocation_entry::get(req_id).expect("Should have data");
        assert_eq!(data.init_duration, 123.45);
    }

    #[test]
    #[serial]
    fn test_parse_platform_runtime_done() {
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
        process_telemetry_logs(&mut logs);

        let data = invocation_entry::get(req_id).expect("Should have data");
        let expected_end = chrono::DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.end_time, expected_end);
        assert_eq!(data.duration, 500.0);
    }

    #[test]
    #[serial]
    fn test_parse_platform_runtime_done_with_response_duration_span() {
        let req_id = "test-req-done-span";
        let log_time = "2023-01-01T12:00:01.000Z";
        let response_duration_start = "2023-01-01T12:00:00.900Z";
        let logs = vec![create_log(
            "platform.runtimeDone",
            log_time,
            json!({
                "requestId": req_id,
                "spans": [
                    { "name": "responseLatency", "start": "2023-01-01T12:00:00.800Z", "durationMs": 65.0 },
                    { "name": "responseDuration", "start": response_duration_start, "durationMs": 0.077 },
                    { "name": "runtimeOverhead", "start": "2023-01-01T12:00:00.950Z", "durationMs": 1.0 }
                ],
                "metrics": { "durationMs": 500.0 }
            }),
            None,
        )];

        let mut logs = logs;
        process_telemetry_logs(&mut logs);

        let data = invocation_entry::get(req_id).expect("Should have data");
        // Should use responseDuration start, not the log time
        let expected_end = chrono::DateTime::parse_from_rfc3339(response_duration_start)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.end_time, expected_end);
        assert_eq!(data.duration, 500.0);
    }

    #[test]
    #[serial]
    fn test_parse_platform_report() {
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
        process_telemetry_logs(&mut logs);

        let data = invocation_entry::get(req_id).expect("Should have data");
        assert_eq!(data.end_time, 0.0); // report does not set end_time
        assert_eq!(data.duration, 600.0);
        assert_eq!(data.billed_duration, 700.0);
        assert_eq!(data.memory_usage, 128);
        assert_eq!(data.init_duration, 50.0);
    }

    #[test]
    #[serial]
    fn test_process_telemetry_logs_full_flow() {
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

        process_telemetry_logs(&mut logs);

        let data = invocation_entry::get(req_id).expect("Should have data");
        let expected_start = chrono::DateTime::parse_from_rfc3339(start_time)
            .unwrap()
            .timestamp_millis() as f64;

        let expected_end = chrono::DateTime::parse_from_rfc3339(end_time)
            .unwrap()
            .timestamp_millis() as f64;
        assert_eq!(data.start_time, expected_start);
        assert_eq!(data.end_time, expected_end); // set by runtimeDone, not overwritten by report
                                                 // Report duration should overwrite runtimeDone duration
        assert_eq!(data.duration, 1000.1);
        assert_eq!(data.billed_duration, 1001.0);
        assert_eq!(data.memory_usage, 256);
        // init_duration was not set in report (0.0), so check if it remains default (0.0) or if we had set it previously.
        // We didn't set it previously in this flow.
        assert_eq!(data.init_duration, 0.0);
    }
}

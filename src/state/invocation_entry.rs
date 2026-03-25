use std::collections::HashMap;

use once_cell::sync::Lazy;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::trace::v1::Status;
use parking_lot::Mutex;

use super::invocation_data::{StoredTrace, TelemetryLog};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvocationState {
    Pending,
    Done,
}

#[derive(Clone)]
pub struct InvocationEntry {
    pub state: InvocationState,
    pub event_payload: Option<String>,
    pub return_value: Option<String>,
    pub span_id: Option<String>,
    pub root_span_id: Option<String>,
    pub trace_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub sampled: bool,
    pub init_duration: f64,
    pub duration: f64,
    pub billed_duration: f64,
    pub start_time: f64,
    pub end_time: f64,
    pub memory_usage: u64,
    pub handler_attributes: Vec<KeyValue>,
    pub handler_status: Option<Status>,
    pub traces: Vec<StoredTrace>,
    pub logs: Vec<TelemetryLog>,
}

impl Default for InvocationEntry {
    fn default() -> Self {
        Self {
            state: InvocationState::Pending,
            event_payload: None,
            return_value: None,
            span_id: None,
            root_span_id: None,
            trace_id: None,
            parent_span_id: None,
            sampled: false,
            init_duration: 0.0,
            duration: 0.0,
            billed_duration: 0.0,
            start_time: 0.0,
            end_time: 0.0,
            memory_usage: 0,
            handler_attributes: Vec::new(),
            handler_status: None,
            traces: Vec::new(),
            logs: Vec::new(),
        }
    }
}

static INVOCATION_STORE: Lazy<Mutex<HashMap<String, InvocationEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn get_or_create(invocation_id: &str) -> InvocationEntry {
    let mut store = INVOCATION_STORE.lock();
    store.entry(invocation_id.to_string()).or_default().clone()
}

pub fn update<F>(invocation_id: &str, update_fn: F)
where
    F: FnOnce(&mut InvocationEntry),
{
    let mut store = INVOCATION_STORE.lock();
    let entry = store.entry(invocation_id.to_string()).or_default();
    update_fn(entry);
}

pub fn get(invocation_id: &str) -> Option<InvocationEntry> {
    INVOCATION_STORE.lock().get(invocation_id).cloned()
}

/// Lightweight getter: returns only the event_payload, avoiding a full entry clone.
pub fn get_event_payload(invocation_id: &str) -> Option<String> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .and_then(|e| e.event_payload.clone())
}

/// Lightweight getter: returns only the start_time.
pub fn get_start_time(invocation_id: &str) -> Option<f64> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .map(|e| e.start_time)
        .filter(|&t| t > 0.0)
}

/// Lightweight getter: returns only the root_span_id.
pub fn get_root_span_id(invocation_id: &str) -> Option<String> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .and_then(|e| e.root_span_id.clone())
}

/// Atomically returns the root_span_id, generating and storing a new random one if not yet set.
/// This avoids the race where the OTLP receiver processes trace data before the extension
/// events loop has set root_span_id via handle_invoke_event.
pub fn get_or_create_root_span_id(invocation_id: &str) -> String {
    let mut store = INVOCATION_STORE.lock();
    let entry = store.entry(invocation_id.to_string()).or_default();
    entry
        .root_span_id
        .get_or_insert_with(|| hex::encode(crate::util::parsers::generate_random_span_id()))
        .clone()
}

/// Lightweight getter: returns only the sampled flag.
pub fn get_sampled(invocation_id: &str) -> bool {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .map_or(false, |e| e.sampled)
}

/// Lightweight getter: returns only the return_value, avoiding a full entry clone.
pub fn get_return_value(invocation_id: &str) -> Option<String> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .and_then(|e| e.return_value.clone())
}

/// Lightweight getter: returns (trace_id, span_id, parent_span_id) without cloning traces/logs.
pub fn get_trace_span_ids(
    invocation_id: &str,
) -> Option<(Option<String>, Option<String>, Option<String>)> {
    INVOCATION_STORE.lock().get(invocation_id).map(|e| {
        (
            e.trace_id.clone(),
            e.span_id.clone(),
            e.parent_span_id.clone(),
        )
    })
}

/// Lightweight getter: returns (trace_id, root_span_id) for log correlation.
pub fn get_trace_span_ids_for_logs(
    invocation_id: &str,
) -> Option<(Option<String>, Option<String>)> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .map(|e| (e.trace_id.clone(), e.root_span_id.clone()))
}

/// Lightweight getter: returns the telemetry data fields needed for span annotation.
pub struct TelemetryData {
    pub init_duration: f64,
    pub billed_duration: f64,
    pub memory_usage: u64,
    pub start_time: f64,
    pub end_time: f64,
}

/// Data needed to build supplementary spans (root span).
#[derive(Debug)]
pub struct SupplementarySpanData {
    pub root_span_id: Option<String>,
    pub trace_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub sampled: bool,
    pub start_time: f64,
    pub billed_duration: f64,
    pub init_duration: f64,
    pub memory_usage: u64,
    pub end_time: f64,
    pub handler_attributes: Vec<KeyValue>,
    pub handler_status: Option<Status>,
}

pub fn get_supplementary_span_data(invocation_id: &str) -> Option<SupplementarySpanData> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .map(|e| SupplementarySpanData {
            root_span_id: e.root_span_id.clone(),
            trace_id: e.trace_id.clone(),
            parent_span_id: e.parent_span_id.clone(),
            sampled: e.sampled,
            start_time: e.start_time,
            billed_duration: e.billed_duration,
            init_duration: e.init_duration,
            memory_usage: e.memory_usage,
            end_time: e.end_time,
            handler_attributes: e.handler_attributes.clone(),
            handler_status: e.handler_status.clone(),
        })
}

pub fn get_telemetry_data(invocation_id: &str) -> Option<TelemetryData> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .map(|e| TelemetryData {
            init_duration: e.init_duration,
            billed_duration: e.billed_duration,
            memory_usage: e.memory_usage,
            start_time: e.start_time,
            end_time: e.end_time,
        })
}

/// Data needed to build supplementary metrics (histograms).
pub struct MetricsData {
    pub duration: f64,
    pub init_duration: f64,
    pub billed_duration: f64,
    pub memory_usage: u64,
    pub start_time: f64,
    pub end_time: f64,
}

pub fn get_metrics_data(invocation_id: &str) -> Option<MetricsData> {
    INVOCATION_STORE
        .lock()
        .get(invocation_id)
        .map(|e| MetricsData {
            duration: e.duration,
            init_duration: e.init_duration,
            billed_duration: e.billed_duration,
            memory_usage: e.memory_usage,
            start_time: e.start_time,
            end_time: e.end_time,
        })
}

pub fn remove(invocation_id: &str) -> Option<InvocationEntry> {
    INVOCATION_STORE.lock().remove(invocation_id)
}

/// Store telemetry logs, grouping each log into its invocation entry.
/// Logs without an invocation_id are stored under a shared "__unknown__" key.
pub fn store_telemetry_logs(logs: Vec<TelemetryLog>) {
    let mut store = INVOCATION_STORE.lock();
    for log in logs {
        let key = log
            .invocation_id
            .clone()
            .unwrap_or_else(|| "__unknown__".to_string());
        store.entry(key).or_default().logs.push(log);
    }
}

/// Take all telemetry logs from every invocation entry, draining each entry's logs.
/// If `exclude_invocation_id` is provided, logs belonging to that invocation are left in place.
pub fn take_all_telemetry_logs(exclude_invocation_id: Option<&str>) -> Vec<TelemetryLog> {
    let mut store = INVOCATION_STORE.lock();
    let mut all_logs = Vec::new();
    for (key, entry) in store.iter_mut() {
        if exclude_invocation_id.is_some_and(|id| key == id) {
            continue;
        }
        all_logs.append(&mut entry.logs);
    }
    all_logs
}

/// Snapshot all telemetry logs from every invocation entry (non-destructive).
#[allow(dead_code)]
pub fn snapshot_all_telemetry_logs() -> Vec<TelemetryLog> {
    let store = INVOCATION_STORE.lock();
    store
        .values()
        .flat_map(|entry| entry.logs.iter().cloned())
        .collect()
}

/// Take all traces for a specific invocation ID, draining that entry's traces.
pub fn take_traces_by_id(invocation_id: &str) -> Vec<StoredTrace> {
    let mut store = INVOCATION_STORE.lock();
    if let Some(entry) = store.get_mut(invocation_id) {
        std::mem::take(&mut entry.traces)
    } else {
        Vec::new()
    }
}

/// Store a trace under a known invocation ID directly.
pub fn store_trace_by_id(invocation_id: &str, trace: StoredTrace) {
    INVOCATION_STORE
        .lock()
        .entry(invocation_id.to_string())
        .or_default()
        .traces
        .push(trace);
}

/// Take all traces from every invocation entry, draining each entry's traces.
pub fn take_all_traces() -> Vec<StoredTrace> {
    let mut store = INVOCATION_STORE.lock();
    let mut all_traces = Vec::new();
    for entry in store.values_mut() {
        all_traces.append(&mut entry.traces);
    }
    all_traces
}

/// Snapshot all traces from every invocation entry (non-destructive).
#[allow(dead_code)]
pub fn snapshot_all_traces() -> Vec<StoredTrace> {
    let store = INVOCATION_STORE.lock();
    store
        .values()
        .flat_map(|entry| entry.traces.iter().cloned())
        .collect()
}

const MAX_INVOCATION_ENTRIES: usize = 10;

/// Remove all invocation entries whose state is Done and whose traces and logs are empty.
/// Then, if the store still exceeds MAX_INVOCATION_ENTRIES, evict the oldest entries
/// by start_time to keep the store bounded.
pub fn delete_done_invocations() {
    let mut store = INVOCATION_STORE.lock();
    store.retain(|id, entry| {
        let should_delete = entry.state == InvocationState::Done
            && entry.traces.is_empty()
            && entry.logs.is_empty();
        if should_delete {
            tracing::trace!(
                "[{}] Deleting done invocation entry: {}",
                crate::log_prefix(),
                id
            );
        }
        !should_delete
    });

    if store.len() > MAX_INVOCATION_ENTRIES {
        let excess = store.len() - MAX_INVOCATION_ENTRIES;
        let mut entries: Vec<(String, f64)> = store
            .iter()
            .map(|(id, entry)| (id.clone(), entry.start_time))
            .collect();
        entries.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (id, _) in entries.into_iter().take(excess) {
            tracing::warn!(
                "[{}] Evicting old invocation entry to stay within limit: {}",
                crate::log_prefix(),
                id
            );
            store.remove(&id);
        }
    }
}

pub fn force_init() {
    Lazy::force(&INVOCATION_STORE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn reset_store() {
        INVOCATION_STORE.lock().clear();
    }

    fn make_log(invocation_id: Option<&str>) -> TelemetryLog {
        TelemetryLog {
            time: "2024-01-01T00:00:00Z".to_string(),
            r#type: "function".to_string(),
            record: serde_json::json!({"msg": "hello"}),
            invocation_id: invocation_id.map(String::from),
            trace_id: None,
            span_id: None,
            custom_attributes: vec![],
        }
    }

    fn make_trace(invocation_ids: Vec<&str>) -> StoredTrace {
        StoredTrace {
            method: hyper::Method::POST,
            path_and_query: "/v1/traces".to_string(),
            headers: hyper::HeaderMap::new(),
            body: vec![],
            invocation_ids: invocation_ids.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    #[serial]
    fn get_or_create_returns_default_for_new_id() {
        reset_store();
        let entry = get_or_create("inv-1");
        assert_eq!(entry.state, InvocationState::Pending);
        assert_eq!(entry.span_id, None);
        assert_eq!(entry.duration, 0.0);
    }

    #[test]
    #[serial]
    fn update_modifies_entry_in_place() {
        reset_store();
        update("inv-1", |e| {
            e.state = InvocationState::Done;
            e.duration = 42.0;
        });
        let entry = get("inv-1").unwrap();
        assert_eq!(entry.state, InvocationState::Done);
        assert_eq!(entry.duration, 42.0);
    }

    #[test]
    #[serial]
    fn get_returns_none_for_missing_id() {
        reset_store();
        assert!(get("nonexistent").is_none());
    }

    #[test]
    #[serial]
    fn remove_returns_entry_and_deletes_it() {
        reset_store();
        get_or_create("inv-1");
        let removed = remove("inv-1");
        assert!(removed.is_some());
        assert!(get("inv-1").is_none());
    }

    #[test]
    #[serial]
    fn store_telemetry_logs_groups_by_invocation_id() {
        reset_store();
        let logs = vec![
            make_log(Some("inv-1")),
            make_log(Some("inv-2")),
            make_log(None),
        ];
        store_telemetry_logs(logs);

        assert_eq!(get("inv-1").unwrap().logs.len(), 1);
        assert_eq!(get("inv-2").unwrap().logs.len(), 1);
        assert_eq!(get("__unknown__").unwrap().logs.len(), 1);
    }

    #[test]
    #[serial]
    fn take_all_telemetry_logs_drains_and_respects_exclude() {
        reset_store();
        store_telemetry_logs(vec![make_log(Some("inv-1")), make_log(Some("inv-2"))]);

        let taken = take_all_telemetry_logs(Some("inv-1"));
        assert_eq!(taken.len(), 1);
        // inv-1 logs should still be there
        assert_eq!(get("inv-1").unwrap().logs.len(), 1);
        // inv-2 logs should be drained
        assert!(get("inv-2").unwrap().logs.is_empty());
    }

    #[test]
    #[serial]
    fn store_and_take_traces_by_id() {
        reset_store();
        store_trace_by_id("inv-1", make_trace(vec!["inv-1"]));
        store_trace_by_id("inv-1", make_trace(vec!["inv-1"]));

        let traces = take_traces_by_id("inv-1");
        assert_eq!(traces.len(), 2);
        // After take, traces should be drained
        assert!(take_traces_by_id("inv-1").is_empty());
    }

    #[test]
    #[serial]
    fn take_all_traces_drains_all_entries() {
        reset_store();
        store_trace_by_id("inv-1", make_trace(vec!["inv-1"]));
        store_trace_by_id("inv-2", make_trace(vec!["inv-2"]));

        let all = take_all_traces();
        assert_eq!(all.len(), 2);
        assert!(take_all_traces().is_empty());
    }

    #[test]
    #[serial]
    fn delete_done_invocations_removes_only_done_empty_entries() {
        reset_store();
        // Done + empty → should be deleted
        update("inv-done", |e| e.state = InvocationState::Done);
        // Pending → should remain
        get_or_create("inv-pending");
        // Done but has traces → should remain
        update("inv-done-with-traces", |e| e.state = InvocationState::Done);
        store_trace_by_id(
            "inv-done-with-traces",
            make_trace(vec!["inv-done-with-traces"]),
        );

        delete_done_invocations();

        assert!(get("inv-done").is_none());
        assert!(get("inv-pending").is_some());
        assert!(get("inv-done-with-traces").is_some());
    }

    #[test]
    #[serial]
    fn delete_done_invocations_evicts_oldest_when_over_limit() {
        reset_store();
        // Create MAX + 2 entries, all Pending, with ascending start_times
        for i in 0..(MAX_INVOCATION_ENTRIES + 2) {
            update(&format!("inv-{}", i), |e| {
                e.start_time = (i + 1) as f64; // 1.0, 2.0, ...
            });
        }
        assert_eq!(INVOCATION_STORE.lock().len(), MAX_INVOCATION_ENTRIES + 2);

        delete_done_invocations();

        let store = INVOCATION_STORE.lock();
        assert_eq!(store.len(), MAX_INVOCATION_ENTRIES);
        // The two oldest (start_time 1.0 and 2.0) should be evicted
        assert!(
            !store.contains_key("inv-0"),
            "inv-0 (oldest) should be evicted"
        );
        assert!(
            !store.contains_key("inv-1"),
            "inv-1 (second oldest) should be evicted"
        );
        // The newest should remain
        assert!(
            store.contains_key(&format!("inv-{}", MAX_INVOCATION_ENTRIES + 1)),
            "newest entry should remain"
        );
    }

    #[test]
    #[serial]
    fn delete_done_invocations_evicts_zero_start_time_first() {
        reset_store();
        // Create MAX + 1 entries; one has start_time 0 (never got a platform.start)
        update("inv-no-start", |_| {}); // start_time defaults to 0.0
        for i in 1..=MAX_INVOCATION_ENTRIES {
            update(&format!("inv-{}", i), |e| {
                e.start_time = i as f64;
            });
        }

        delete_done_invocations();

        let store = INVOCATION_STORE.lock();
        assert_eq!(store.len(), MAX_INVOCATION_ENTRIES);
        assert!(
            !store.contains_key("inv-no-start"),
            "entry with start_time=0 should be evicted first"
        );
    }
}

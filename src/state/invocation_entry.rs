use std::collections::HashMap;

use once_cell::sync::Lazy;
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
    pub trace_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub init_duration: f64,
    pub duration: f64,
    pub billed_duration: f64,
    pub start_time: f64,
    pub end_time: f64,
    pub memory_usage: u64,
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
            trace_id: None,
            parent_span_id: None,
            init_duration: 0.0,
            duration: 0.0,
            billed_duration: 0.0,
            start_time: 0.0,
            end_time: 0.0,
            memory_usage: 0,
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
pub fn take_all_telemetry_logs() -> Vec<TelemetryLog> {
    let mut store = INVOCATION_STORE.lock();
    let mut all_logs = Vec::new();
    for entry in store.values_mut() {
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

/// Store a trace under a known invocation ID directly.
pub fn store_trace_by_id(invocation_id: &str, trace: StoredTrace) {
    INVOCATION_STORE
        .lock()
        .entry(invocation_id.to_string())
        .or_default()
        .traces
        .push(trace);
}

/// Store a single trace, filing it under each of its invocation IDs.
/// If the trace has no invocation IDs, it is stored under "__unknown__".
pub fn store_trace(trace: StoredTrace) {
    let mut store = INVOCATION_STORE.lock();
    if trace.invocation_ids.is_empty() {
        store
            .entry("__unknown__".to_string())
            .or_default()
            .traces
            .push(trace);
    } else {
        // For a single invocation id (common case), move the trace directly.
        // For multiple, clone for all but the last.
        let mut ids = trace.invocation_ids.iter();
        let last_id = ids.next_back().unwrap();
        for id in ids {
            store
                .entry(id.clone())
                .or_default()
                .traces
                .push(trace.clone());
        }
        store
            .entry(last_id.clone())
            .or_default()
            .traces
            .push(trace);
    }
}

/// Store multiple traces.
pub fn store_traces(traces: Vec<StoredTrace>) {
    for trace in traces {
        store_trace(trace);
    }
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

pub fn force_init() {
    Lazy::force(&INVOCATION_STORE);
}

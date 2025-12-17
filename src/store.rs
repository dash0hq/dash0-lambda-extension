use std::collections::HashMap;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub fn take_event_payload(invocation_id: &str) -> Option<String> {
    EVENT_PAYLOADS.lock().remove(invocation_id)
}

pub fn get_event_payload(invocation_id: &str) -> Option<String> {
    EVENT_PAYLOADS.lock().get(invocation_id).cloned()
}

pub fn store_event_payload(invocation_id: &str, payload: &str) {
    EVENT_PAYLOADS
        .lock()
        .insert(invocation_id.to_string(), payload.to_string());
}

static EVENT_PAYLOADS: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn store_invocation_start(invocation_id: &str, nanos: u64) {
    INVOCATION_STARTS
        .lock()
        .insert(invocation_id.to_string(), nanos);
}

pub fn get_invocation_start(invocation_id: &str) -> Option<u64> {
    INVOCATION_STARTS.lock().get(invocation_id).cloned()
}

pub fn take_invocation_start(invocation_id: &str) -> Option<u64> {
    INVOCATION_STARTS.lock().remove(invocation_id)
}

static INVOCATION_STARTS: Lazy<Mutex<HashMap<String, u64>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn store_trace(trace: StoredTrace) {
    TRACE_STORE.lock().push(trace);
}

pub fn store_traces(traces: Vec<StoredTrace>) {
    TRACE_STORE.lock().extend(traces);
}

pub fn take_traces() -> Vec<StoredTrace> {
    std::mem::take(&mut *TRACE_STORE.lock())
}

#[allow(dead_code)]
pub fn snapshot_traces() -> Vec<StoredTrace> {
    TRACE_STORE.lock().clone()
}

#[derive(Clone)]
pub struct StoredTrace {
    pub method: hyper::Method,
    pub path_and_query: String,
    pub headers: hyper::HeaderMap,
    pub body: Vec<u8>,
    pub invocation_ids: Vec<String>,
}

pub fn force_init_trace_store() {
    Lazy::force(&TRACE_STORE);
}

static TRACE_STORE: Lazy<Mutex<Vec<StoredTrace>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn store_return_payload(invocation_id: &str, payload: &str) {
    RETURN_PAYLOADS
        .lock()
        .insert(invocation_id.to_string(), payload.to_string());
}

pub fn take_return_payload(invocation_id: &str) -> Option<String> {
    RETURN_PAYLOADS.lock().remove(invocation_id)
}

static RETURN_PAYLOADS: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static CURRENT_INVOCATION_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

pub fn store_current_invocation_id(invocation_id: &str) {
    *CURRENT_INVOCATION_ID.lock() = Some(invocation_id.to_string());
}

pub fn get_current_invocation_id() -> Option<String> {
    CURRENT_INVOCATION_ID.lock().clone()
}

static LAST_SEEN_INVOCATION_START: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

pub fn store_last_seen_invocation_start(invocation_id: &str) {
    *LAST_SEEN_INVOCATION_START.lock() = Some(invocation_id.to_string());
}

pub fn get_last_seen_invocation_start() -> Option<String> {
    LAST_SEEN_INVOCATION_START.lock().clone()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TelemetryLog {
    pub time: String,
    pub r#type: String,
    pub record: serde_json::Value,
    #[serde(skip_deserializing)]
    pub invocation_id: Option<String>,
}

pub fn store_telemetry_logs(logs: Vec<TelemetryLog>) {
    TELEMETRY_LOGS.lock().extend(logs);
}

#[allow(dead_code)]
pub fn get_telemetry_logs() -> Vec<TelemetryLog> {
    TELEMETRY_LOGS.lock().clone()
}

pub fn take_telemetry_logs() -> Vec<TelemetryLog> {
    std::mem::take(&mut *TELEMETRY_LOGS.lock())
}

static TELEMETRY_LOGS: Lazy<Mutex<Vec<TelemetryLog>>> = Lazy::new(|| Mutex::new(Vec::new()));

#[derive(Clone, Debug, PartialEq)]
pub struct SpanId {
    pub trace_id: String,
    pub span_id: String,
}

pub fn store_invocation_span_id(invocation_id: &str, trace_id: String, span_id: String) {
    INVOCATION_SPAN_IDS
        .lock()
        .insert(invocation_id.to_string(), SpanId { trace_id, span_id });
}

pub fn get_invocation_span_id(invocation_id: &str) -> Option<SpanId> {
    INVOCATION_SPAN_IDS.lock().get(invocation_id).cloned()
}

static INVOCATION_SPAN_IDS: Lazy<Mutex<HashMap<String, SpanId>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn store_runtime_done_notifier(sender: tokio::sync::oneshot::Sender<()>) {
    *RUNTIME_DONE_NOTIFIER.lock() = Some(sender);
}

pub fn take_runtime_done_notifier() -> Option<tokio::sync::oneshot::Sender<()>> {
    RUNTIME_DONE_NOTIFIER.lock().take()
}

static RUNTIME_DONE_NOTIFIER: Lazy<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
    Lazy::new(|| Mutex::new(None));

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct InvocationData {
    pub init_duration: f64,
    pub duration: f64,
    pub billed_duration: f64,
    pub start_time: f64,
    pub end_time: f64,
    pub memory_usage: u64,
}

pub fn update_invocation_data<F>(invocation_id: &str, update_fn: F)
where
    F: FnOnce(&mut InvocationData),
{
    let mut store = INVOCATION_DATA.lock();
    let data = store
        .entry(invocation_id.to_string())
        .or_insert_with(InvocationData::default);
    update_fn(data);
}

pub fn get_invocation_data(invocation_id: &str) -> Option<InvocationData> {
    INVOCATION_DATA.lock().get(invocation_id).cloned()
}

static INVOCATION_DATA: Lazy<Mutex<HashMap<String, InvocationData>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn take_invocation_data(invocation_id: &str) -> Option<InvocationData> {
    INVOCATION_DATA.lock().remove(invocation_id)
}

pub(crate) fn cleanup_invocation(invocation_id: &str) {
    take_event_payload(invocation_id);
    take_invocation_start(invocation_id);
    take_return_payload(invocation_id);
    take_invocation_data(invocation_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ... tests ...

    #[test]
    #[serial]
    fn test_store_telemetry_logs() {
        let logs = vec![
            TelemetryLog {
                time: "2025-01-01".to_string(),
                r#type: "test".to_string(),
                record: serde_json::Value::String("rec1".to_string()),
                invocation_id: None,
            },
            TelemetryLog {
                time: "2025-01-02".to_string(),
                r#type: "test2".to_string(),
                record: serde_json::json!({"foo": "bar"}),
                invocation_id: Some("id1".to_string()),
            },
        ];
        store_telemetry_logs(logs.clone());
        let stored = get_telemetry_logs();
        assert!(stored.len() >= 2);
    }

    // ============================================================================
    // Tests for event payload storage functions
    // ============================================================================

    #[test]
    #[serial]
    fn test_store_and_get_event_payload() {
        let invocation_id = "test-invocation-store-get";
        let payload = r#"{"test": "data"}"#;

        store_event_payload(invocation_id, payload);
        let result = get_event_payload(invocation_id);

        assert_eq!(result, Some(payload.to_string()));
    }

    #[test]
    #[serial]
    fn test_get_event_payload_not_found() {
        let result = get_event_payload("non-existent-invocation-id");
        assert_eq!(result, None);
    }

    #[test]
    #[serial]
    fn test_take_event_payload_removes_entry() {
        let invocation_id = "test-invocation-take";
        let payload = r#"{"test": "data to take"}"#;

        store_event_payload(invocation_id, payload);

        // First take should return the payload
        let result1 = take_event_payload(invocation_id);
        assert_eq!(result1, Some(payload.to_string()));

        // Second take should return None (already removed)
        let result2 = take_event_payload(invocation_id);
        assert_eq!(result2, None);

        // get should also return None
        let result3 = get_event_payload(invocation_id);
        assert_eq!(result3, None);
    }

    #[test]
    #[serial]
    fn test_store_overwrites_existing_payload() {
        let invocation_id = "test-invocation-overwrite";
        let payload1 = r#"{"first": "payload"}"#;
        let payload2 = r#"{"second": "payload"}"#;

        store_event_payload(invocation_id, payload1);
        store_event_payload(invocation_id, payload2);

        let result = get_event_payload(invocation_id);
        assert_eq!(result, Some(payload2.to_string()));
    }

    // ============================================================================
    // Tests for invocation data storage functions
    // ============================================================================

    #[test]
    #[serial]
    fn test_update_invocation_data_creates_new() {
        let invocation_id = "test-inv-data-create";

        update_invocation_data(invocation_id, |data| {
            data.start_time = 100.0;
        });

        let result = get_invocation_data(invocation_id).expect("Should exist");
        assert_eq!(result.start_time, 100.0);
        // Check default values
        assert_eq!(result.duration, 0.0);
    }

    #[test]
    #[serial]
    fn test_update_invocation_data_updates_existing_and_preserves() {
        let invocation_id = "test-inv-data-update";

        // First update: set start_time
        update_invocation_data(invocation_id, |data| {
            data.start_time = 100.0;
        });

        // Second update: set duration, verifying start_time is preserved
        update_invocation_data(invocation_id, |data| {
            data.duration = 50.0;
        });

        let result = get_invocation_data(invocation_id).expect("Should exist");
        assert_eq!(result.start_time, 100.0);
        assert_eq!(result.duration, 50.0);
    }
}

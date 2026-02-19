use std::collections::HashMap;
use std::time::Instant;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

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

#[derive(Clone)]
pub struct StoredLog {
    pub method: hyper::Method,
    pub path_and_query: String,
    pub headers: hyper::HeaderMap,
    pub body: Vec<u8>,
}

pub fn store_log(log: StoredLog) {
    LOG_STORE.lock().push(log);
}

pub fn take_logs() -> Vec<StoredLog> {
    std::mem::take(&mut *LOG_STORE.lock())
}

#[allow(dead_code)]
pub fn snapshot_logs() -> Vec<StoredLog> {
    LOG_STORE.lock().clone()
}

static LOG_STORE: Lazy<Mutex<Vec<StoredLog>>> = Lazy::new(|| Mutex::new(Vec::new()));

#[derive(Clone)]
pub struct StoredMetric {
    pub method: hyper::Method,
    pub path_and_query: String,
    pub headers: hyper::HeaderMap,
    pub body: Vec<u8>,
}

pub fn store_metric(metric: StoredMetric) {
    METRIC_STORE.lock().push(metric);
}

pub fn take_metrics() -> Vec<StoredMetric> {
    std::mem::take(&mut *METRIC_STORE.lock())
}

#[allow(dead_code)]
pub fn snapshot_metrics() -> Vec<StoredMetric> {
    METRIC_STORE.lock().clone()
}

static METRIC_STORE: Lazy<Mutex<Vec<StoredMetric>>> = Lazy::new(|| Mutex::new(Vec::new()));

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

#[derive(Clone, Debug)]
pub struct SpanId {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub timestamp: Instant,
}

impl PartialEq for SpanId {
    fn eq(&self, other: &Self) -> bool {
        self.trace_id == other.trace_id
            && self.span_id == other.span_id
            && self.parent_span_id == other.parent_span_id
    }
}

const MAX_INVOCATION_SPAN_IDS: usize = 10;

pub fn store_invocation_span_id(
    invocation_id: &str,
    trace_id: String,
    span_id: String,
    parent_span_id: String,
) {
    let mut store = INVOCATION_SPAN_IDS.lock();

    // If we're at capacity and this is a new key, remove the oldest entry
    if store.len() >= MAX_INVOCATION_SPAN_IDS && !store.contains_key(invocation_id) {
        if let Some(oldest_key) = store
            .iter()
            .min_by_key(|(_, v)| v.timestamp)
            .map(|(k, _)| k.clone())
        {
            store.remove(&oldest_key);
        }
    }

    store.insert(
        invocation_id.to_string(),
        SpanId {
            trace_id,
            span_id,
            parent_span_id,
            timestamp: Instant::now(),
        },
    );
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

    // ============================================================================
    // Tests for invocation span ID storage functions
    // ============================================================================

    #[cfg(test)]
    fn clear_invocation_span_ids() {
        INVOCATION_SPAN_IDS.lock().clear();
    }

    #[cfg(test)]
    fn invocation_span_ids_len() -> usize {
        INVOCATION_SPAN_IDS.lock().len()
    }

    #[test]
    #[serial]
    fn test_invocation_span_ids_max_capacity() {
        clear_invocation_span_ids();

        // Store 10 span IDs with small delays to ensure different timestamps
        for i in 0..10 {
            store_invocation_span_id(
                &format!("inv-{}", i),
                format!("trace-{}", i),
                format!("span-{}", i),
                String::new(),
            );
            // Small sleep to ensure distinct timestamps
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        // Verify all 10 are present
        assert_eq!(invocation_span_ids_len(), 10);
        for i in 0..10 {
            assert!(
                get_invocation_span_id(&format!("inv-{}", i)).is_some(),
                "inv-{} should exist",
                i
            );
        }

        // Add an 11th item
        store_invocation_span_id(
            "inv-10",
            "trace-10".to_string(),
            "span-10".to_string(),
            String::new(),
        );

        // Verify the map still has only 10 items
        assert_eq!(invocation_span_ids_len(), 10);

        // Verify the oldest one (inv-0) was removed
        assert!(
            get_invocation_span_id("inv-0").is_none(),
            "inv-0 should have been evicted"
        );

        // Verify the newest one exists
        assert!(
            get_invocation_span_id("inv-10").is_some(),
            "inv-10 should exist"
        );

        // Verify items 1-9 still exist
        for i in 1..10 {
            assert!(
                get_invocation_span_id(&format!("inv-{}", i)).is_some(),
                "inv-{} should still exist",
                i
            );
        }
    }

    #[test]
    #[serial]
    fn test_invocation_span_ids_update_existing_does_not_evict() {
        clear_invocation_span_ids();

        // Store 10 span IDs
        for i in 0..10 {
            store_invocation_span_id(
                &format!("inv-{}", i),
                format!("trace-{}", i),
                format!("span-{}", i),
                String::new(),
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(invocation_span_ids_len(), 10);

        // Update an existing key (should not evict anything)
        store_invocation_span_id(
            "inv-0",
            "trace-0-updated".to_string(),
            "span-0-updated".to_string(),
            String::new(),
        );

        // Verify still 10 items
        assert_eq!(invocation_span_ids_len(), 10);

        // Verify the update took effect
        let updated = get_invocation_span_id("inv-0").expect("inv-0 should exist");
        assert_eq!(updated.trace_id, "trace-0-updated");
        assert_eq!(updated.span_id, "span-0-updated");

        // Verify all others still exist
        for i in 1..10 {
            assert!(
                get_invocation_span_id(&format!("inv-{}", i)).is_some(),
                "inv-{} should still exist",
                i
            );
        }
    }
}

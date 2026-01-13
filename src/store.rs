use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

// ============================================================================
// PayloadValue - Unified return type for String and Arc<String> variants
// ============================================================================

#[derive(Clone, Debug)]
pub enum PayloadValue {
    String(String),
    Arc(Arc<String>),
}

impl PayloadValue {
    pub fn as_str(&self) -> &str {
        match self {
            Self::String(s) => s.as_str(),
            Self::Arc(a) => a.as_str(),
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Arc(a) => (**a).clone(),
        }
    }
}

impl PartialEq for PayloadValue {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<String> for PayloadValue {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<&str> for PayloadValue {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

// ============================================================================
// EVENT_PAYLOADS - Quad implementation (String/Arc x Mutex/RwLock)
// ============================================================================

// String-based stores
static EVENT_PAYLOADS_STRING_MUTEX: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static EVENT_PAYLOADS_STRING_RWLOCK: Lazy<tokio::sync::RwLock<HashMap<String, String>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

// Arc-based stores
static EVENT_PAYLOADS_ARC_MUTEX: Lazy<Mutex<HashMap<String, Arc<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static EVENT_PAYLOADS_ARC_RWLOCK: Lazy<tokio::sync::RwLock<HashMap<String, Arc<String>>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

pub async fn take_event_payload(invocation_id: &str) -> Option<PayloadValue> {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        if config.use_tokio_rwlock {
            EVENT_PAYLOADS_ARC_RWLOCK
                .write()
                .await
                .remove(invocation_id)
                .map(PayloadValue::Arc)
        } else {
            EVENT_PAYLOADS_ARC_MUTEX
                .lock()
                .remove(invocation_id)
                .map(PayloadValue::Arc)
        }
    } else {
        if config.use_tokio_rwlock {
            EVENT_PAYLOADS_STRING_RWLOCK
                .write()
                .await
                .remove(invocation_id)
                .map(PayloadValue::String)
        } else {
            EVENT_PAYLOADS_STRING_MUTEX
                .lock()
                .remove(invocation_id)
                .map(PayloadValue::String)
        }
    }
}

pub async fn get_event_payload(invocation_id: &str) -> Option<PayloadValue> {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        if config.use_tokio_rwlock {
            EVENT_PAYLOADS_ARC_RWLOCK
                .read()
                .await
                .get(invocation_id)
                .cloned()
                .map(PayloadValue::Arc)
        } else {
            EVENT_PAYLOADS_ARC_MUTEX
                .lock()
                .get(invocation_id)
                .cloned()
                .map(PayloadValue::Arc)
        }
    } else {
        if config.use_tokio_rwlock {
            EVENT_PAYLOADS_STRING_RWLOCK
                .read()
                .await
                .get(invocation_id)
                .cloned()
                .map(PayloadValue::String)
        } else {
            EVENT_PAYLOADS_STRING_MUTEX
                .lock()
                .get(invocation_id)
                .cloned()
                .map(PayloadValue::String)
        }
    }
}

pub async fn store_event_payload(invocation_id: &str, payload: &str) {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        let arc_payload = Arc::new(payload.to_string());
        if config.use_tokio_rwlock {
            EVENT_PAYLOADS_ARC_RWLOCK
                .write()
                .await
                .insert(invocation_id.to_string(), arc_payload);
        } else {
            EVENT_PAYLOADS_ARC_MUTEX
                .lock()
                .insert(invocation_id.to_string(), arc_payload);
        }
    } else {
        if config.use_tokio_rwlock {
            EVENT_PAYLOADS_STRING_RWLOCK
                .write()
                .await
                .insert(invocation_id.to_string(), payload.to_string());
        } else {
            EVENT_PAYLOADS_STRING_MUTEX
                .lock()
                .insert(invocation_id.to_string(), payload.to_string());
        }
    }
}

// ============================================================================
// TRACE_STORE - Dual implementation for Mutex and RwLock
// ============================================================================

static TRACE_STORE_MUTEX: Lazy<Mutex<Vec<StoredTrace>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

static TRACE_STORE_RWLOCK: Lazy<tokio::sync::RwLock<Vec<StoredTrace>>> =
    Lazy::new(|| tokio::sync::RwLock::new(Vec::new()));

pub async fn store_trace(trace: StoredTrace) {
    if crate::config::performance::get_config().use_tokio_rwlock {
        TRACE_STORE_RWLOCK.write().await.push(trace);
    } else {
        TRACE_STORE_MUTEX.lock().push(trace);
    }
}

pub async fn store_traces(traces: Vec<StoredTrace>) {
    if crate::config::performance::get_config().use_tokio_rwlock {
        TRACE_STORE_RWLOCK.write().await.extend(traces);
    } else {
        TRACE_STORE_MUTEX.lock().extend(traces);
    }
}

pub async fn take_traces() -> Vec<StoredTrace> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        std::mem::take(&mut *TRACE_STORE_RWLOCK.write().await)
    } else {
        std::mem::take(&mut *TRACE_STORE_MUTEX.lock())
    }
}

#[allow(dead_code)]
pub async fn snapshot_traces() -> Vec<StoredTrace> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        TRACE_STORE_RWLOCK.read().await.clone()
    } else {
        TRACE_STORE_MUTEX.lock().clone()
    }
}

#[derive(Clone)]
pub struct StoredTrace {
    pub method: hyper::Method,
    pub path_and_query: String,
    pub headers: hyper::HeaderMap,
    pub body: Vec<u8>,
    pub invocation_ids: Vec<String>,
    // Lazy decode optimization: cache decoded protobuf to avoid redundant decode/encode
    decoded: Option<opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest>,
}

impl StoredTrace {
    /// Create a new StoredTrace without decoded cache
    pub fn new(
        method: hyper::Method,
        path_and_query: String,
        headers: hyper::HeaderMap,
        body: Vec<u8>,
        invocation_ids: Vec<String>,
    ) -> Self {
        Self {
            method,
            path_and_query,
            headers,
            body,
            invocation_ids,
            decoded: None,
        }
    }

    /// Decode the body if not already decoded (lazy decode optimization)
    pub fn decode_if_needed(&mut self) -> Result<&mut opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest, prost::DecodeError> {
        use prost::Message;

        if self.decoded.is_none() {
            self.decoded = Some(
                opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest::decode(
                    self.body.as_slice()
                )?
            );
        }
        Ok(self.decoded.as_mut().unwrap())
    }

    /// Re-encode the body if it was decoded and potentially modified
    pub fn encode_if_modified(&mut self) -> Result<(), prost::EncodeError> {
        use prost::Message;

        if let Some(decoded) = &self.decoded {
            self.body = decoded.encode_to_vec();
        }
        Ok(())
    }

    /// Get a reference to the decoded trace if it exists
    pub fn get_decoded(&self) -> Option<&opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest> {
        self.decoded.as_ref()
    }
}

pub fn force_init_trace_store() {
    if crate::config::performance::get_config().use_tokio_rwlock {
        Lazy::force(&TRACE_STORE_RWLOCK);
    } else {
        Lazy::force(&TRACE_STORE_MUTEX);
    }
}

// ============================================================================
// RETURN_PAYLOADS - Quad implementation (String/Arc x Mutex/RwLock)
// ============================================================================

// String-based stores
static RETURN_PAYLOADS_STRING_MUTEX: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static RETURN_PAYLOADS_STRING_RWLOCK: Lazy<tokio::sync::RwLock<HashMap<String, String>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

// Arc-based stores
static RETURN_PAYLOADS_ARC_MUTEX: Lazy<Mutex<HashMap<String, Arc<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static RETURN_PAYLOADS_ARC_RWLOCK: Lazy<tokio::sync::RwLock<HashMap<String, Arc<String>>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

pub async fn store_return_payload(invocation_id: &str, payload: &str) {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        let arc_payload = Arc::new(payload.to_string());
        if config.use_tokio_rwlock {
            RETURN_PAYLOADS_ARC_RWLOCK
                .write()
                .await
                .insert(invocation_id.to_string(), arc_payload);
        } else {
            RETURN_PAYLOADS_ARC_MUTEX
                .lock()
                .insert(invocation_id.to_string(), arc_payload);
        }
    } else {
        if config.use_tokio_rwlock {
            RETURN_PAYLOADS_STRING_RWLOCK
                .write()
                .await
                .insert(invocation_id.to_string(), payload.to_string());
        } else {
            RETURN_PAYLOADS_STRING_MUTEX
                .lock()
                .insert(invocation_id.to_string(), payload.to_string());
        }
    }
}

pub async fn take_return_payload(invocation_id: &str) -> Option<PayloadValue> {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        if config.use_tokio_rwlock {
            RETURN_PAYLOADS_ARC_RWLOCK
                .write()
                .await
                .remove(invocation_id)
                .map(PayloadValue::Arc)
        } else {
            RETURN_PAYLOADS_ARC_MUTEX
                .lock()
                .remove(invocation_id)
                .map(PayloadValue::Arc)
        }
    } else {
        if config.use_tokio_rwlock {
            RETURN_PAYLOADS_STRING_RWLOCK
                .write()
                .await
                .remove(invocation_id)
                .map(PayloadValue::String)
        } else {
            RETURN_PAYLOADS_STRING_MUTEX
                .lock()
                .remove(invocation_id)
                .map(PayloadValue::String)
        }
    }
}

// ============================================================================
// CURRENT_INVOCATION_ID - Quad implementation (String/Arc x Mutex/RwLock)
// ============================================================================

// String-based stores
static CURRENT_INVOCATION_ID_STRING_MUTEX: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));

static CURRENT_INVOCATION_ID_STRING_RWLOCK: Lazy<tokio::sync::RwLock<Option<String>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));

// Arc-based stores
static CURRENT_INVOCATION_ID_ARC_MUTEX: Lazy<Mutex<Option<Arc<String>>>> =
    Lazy::new(|| Mutex::new(None));

static CURRENT_INVOCATION_ID_ARC_RWLOCK: Lazy<tokio::sync::RwLock<Option<Arc<String>>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));

pub async fn store_current_invocation_id(invocation_id: &str) {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        let arc_id = Arc::new(invocation_id.to_string());
        if config.use_tokio_rwlock {
            *CURRENT_INVOCATION_ID_ARC_RWLOCK.write().await = Some(arc_id);
        } else {
            *CURRENT_INVOCATION_ID_ARC_MUTEX.lock() = Some(arc_id);
        }
    } else {
        if config.use_tokio_rwlock {
            *CURRENT_INVOCATION_ID_STRING_RWLOCK.write().await = Some(invocation_id.to_string());
        } else {
            *CURRENT_INVOCATION_ID_STRING_MUTEX.lock() = Some(invocation_id.to_string());
        }
    }
}

pub async fn get_current_invocation_id() -> Option<PayloadValue> {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        if config.use_tokio_rwlock {
            CURRENT_INVOCATION_ID_ARC_RWLOCK
                .read()
                .await
                .clone()
                .map(PayloadValue::Arc)
        } else {
            CURRENT_INVOCATION_ID_ARC_MUTEX
                .lock()
                .clone()
                .map(PayloadValue::Arc)
        }
    } else {
        if config.use_tokio_rwlock {
            CURRENT_INVOCATION_ID_STRING_RWLOCK
                .read()
                .await
                .clone()
                .map(PayloadValue::String)
        } else {
            CURRENT_INVOCATION_ID_STRING_MUTEX
                .lock()
                .clone()
                .map(PayloadValue::String)
        }
    }
}

// ============================================================================
// LAST_SEEN_INVOCATION_START - Quad implementation (String/Arc x Mutex/RwLock)
// ============================================================================

// String-based stores
static LAST_SEEN_INVOCATION_START_STRING_MUTEX: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));

static LAST_SEEN_INVOCATION_START_STRING_RWLOCK: Lazy<tokio::sync::RwLock<Option<String>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));

// Arc-based stores
static LAST_SEEN_INVOCATION_START_ARC_MUTEX: Lazy<Mutex<Option<Arc<String>>>> =
    Lazy::new(|| Mutex::new(None));

static LAST_SEEN_INVOCATION_START_ARC_RWLOCK: Lazy<tokio::sync::RwLock<Option<Arc<String>>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));

pub async fn store_last_seen_invocation_start(invocation_id: &str) {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        let arc_id = Arc::new(invocation_id.to_string());
        if config.use_tokio_rwlock {
            *LAST_SEEN_INVOCATION_START_ARC_RWLOCK.write().await = Some(arc_id);
        } else {
            *LAST_SEEN_INVOCATION_START_ARC_MUTEX.lock() = Some(arc_id);
        }
    } else {
        if config.use_tokio_rwlock {
            *LAST_SEEN_INVOCATION_START_STRING_RWLOCK.write().await = Some(invocation_id.to_string());
        } else {
            *LAST_SEEN_INVOCATION_START_STRING_MUTEX.lock() = Some(invocation_id.to_string());
        }
    }
}

pub async fn get_last_seen_invocation_start() -> Option<PayloadValue> {
    let config = crate::config::performance::get_config();

    if config.use_arc_strings {
        if config.use_tokio_rwlock {
            LAST_SEEN_INVOCATION_START_ARC_RWLOCK
                .read()
                .await
                .clone()
                .map(PayloadValue::Arc)
        } else {
            LAST_SEEN_INVOCATION_START_ARC_MUTEX
                .lock()
                .clone()
                .map(PayloadValue::Arc)
        }
    } else {
        if config.use_tokio_rwlock {
            LAST_SEEN_INVOCATION_START_STRING_RWLOCK
                .read()
                .await
                .clone()
                .map(PayloadValue::String)
        } else {
            LAST_SEEN_INVOCATION_START_STRING_MUTEX
                .lock()
                .clone()
                .map(PayloadValue::String)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TelemetryLog {
    pub time: String,
    pub r#type: String,
    pub record: serde_json::Value,
    #[serde(skip_deserializing)]
    pub invocation_id: Option<String>,
}

// ============================================================================
// TELEMETRY_LOGS - Dual implementation for Mutex and RwLock
// ============================================================================

static TELEMETRY_LOGS_MUTEX: Lazy<Mutex<Vec<TelemetryLog>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

static TELEMETRY_LOGS_RWLOCK: Lazy<tokio::sync::RwLock<Vec<TelemetryLog>>> =
    Lazy::new(|| tokio::sync::RwLock::new(Vec::new()));

pub async fn store_telemetry_logs(logs: Vec<TelemetryLog>) {
    if crate::config::performance::get_config().use_tokio_rwlock {
        TELEMETRY_LOGS_RWLOCK.write().await.extend(logs);
    } else {
        TELEMETRY_LOGS_MUTEX.lock().extend(logs);
    }
}

#[allow(dead_code)]
pub async fn get_telemetry_logs() -> Vec<TelemetryLog> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        TELEMETRY_LOGS_RWLOCK.read().await.clone()
    } else {
        TELEMETRY_LOGS_MUTEX.lock().clone()
    }
}

pub async fn take_telemetry_logs() -> Vec<TelemetryLog> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        std::mem::take(&mut *TELEMETRY_LOGS_RWLOCK.write().await)
    } else {
        std::mem::take(&mut *TELEMETRY_LOGS_MUTEX.lock())
    }
}

#[derive(Clone, Debug)]
pub struct SpanId {
    pub trace_id: String,
    pub span_id: String,
    pub timestamp: Instant,
}

impl PartialEq for SpanId {
    fn eq(&self, other: &Self) -> bool {
        self.trace_id == other.trace_id && self.span_id == other.span_id
    }
}

const MAX_INVOCATION_SPAN_IDS: usize = 10;

// ============================================================================
// INVOCATION_SPAN_IDS - Dual implementation for Mutex and RwLock
// ============================================================================

static INVOCATION_SPAN_IDS_MUTEX: Lazy<Mutex<HashMap<String, SpanId>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static INVOCATION_SPAN_IDS_RWLOCK: Lazy<tokio::sync::RwLock<HashMap<String, SpanId>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

pub async fn store_invocation_span_id(invocation_id: &str, trace_id: String, span_id: String) {
    if crate::config::performance::get_config().use_tokio_rwlock {
        let mut store = INVOCATION_SPAN_IDS_RWLOCK.write().await;

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
                timestamp: Instant::now(),
            },
        );
    } else {
        let mut store = INVOCATION_SPAN_IDS_MUTEX.lock();

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
                timestamp: Instant::now(),
            },
        );
    }
}

pub async fn get_invocation_span_id(invocation_id: &str) -> Option<SpanId> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        INVOCATION_SPAN_IDS_RWLOCK
            .read()
            .await
            .get(invocation_id)
            .cloned()
    } else {
        INVOCATION_SPAN_IDS_MUTEX.lock().get(invocation_id).cloned()
    }
}

// ============================================================================
// RUNTIME_DONE_NOTIFIER - Dual implementation for Mutex and RwLock
// ============================================================================

static RUNTIME_DONE_NOTIFIER_MUTEX: Lazy<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
    Lazy::new(|| Mutex::new(None));

static RUNTIME_DONE_NOTIFIER_RWLOCK: Lazy<tokio::sync::RwLock<Option<tokio::sync::oneshot::Sender<()>>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));

pub async fn store_runtime_done_notifier(sender: tokio::sync::oneshot::Sender<()>) {
    if crate::config::performance::get_config().use_tokio_rwlock {
        *RUNTIME_DONE_NOTIFIER_RWLOCK.write().await = Some(sender);
    } else {
        *RUNTIME_DONE_NOTIFIER_MUTEX.lock() = Some(sender);
    }
}

pub async fn take_runtime_done_notifier() -> Option<tokio::sync::oneshot::Sender<()>> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        RUNTIME_DONE_NOTIFIER_RWLOCK.write().await.take()
    } else {
        RUNTIME_DONE_NOTIFIER_MUTEX.lock().take()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct InvocationData {
    pub init_duration: f64,
    pub duration: f64,
    pub billed_duration: f64,
    pub start_time: f64,
    pub end_time: f64,
    pub memory_usage: u64,
}

// ============================================================================
// INVOCATION_DATA - Dual implementation for Mutex and RwLock
// ============================================================================

static INVOCATION_DATA_MUTEX: Lazy<Mutex<HashMap<String, InvocationData>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static INVOCATION_DATA_RWLOCK: Lazy<tokio::sync::RwLock<HashMap<String, InvocationData>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

pub async fn update_invocation_data<F>(invocation_id: &str, update_fn: F)
where
    F: FnOnce(&mut InvocationData),
{
    if crate::config::performance::get_config().use_tokio_rwlock {
        let mut store = INVOCATION_DATA_RWLOCK.write().await;
        let data = store
            .entry(invocation_id.to_string())
            .or_insert_with(InvocationData::default);
        update_fn(data);
    } else {
        let mut store = INVOCATION_DATA_MUTEX.lock();
        let data = store
            .entry(invocation_id.to_string())
            .or_insert_with(InvocationData::default);
        update_fn(data);
    }
}

pub async fn get_invocation_data(invocation_id: &str) -> Option<InvocationData> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        INVOCATION_DATA_RWLOCK
            .read()
            .await
            .get(invocation_id)
            .cloned()
    } else {
        INVOCATION_DATA_MUTEX.lock().get(invocation_id).cloned()
    }
}

pub async fn take_invocation_data(invocation_id: &str) -> Option<InvocationData> {
    if crate::config::performance::get_config().use_tokio_rwlock {
        INVOCATION_DATA_RWLOCK.write().await.remove(invocation_id)
    } else {
        INVOCATION_DATA_MUTEX.lock().remove(invocation_id)
    }
}

pub(crate) async fn cleanup_invocation(invocation_id: &str) {
    take_event_payload(invocation_id).await;
    take_return_payload(invocation_id).await;
    take_invocation_data(invocation_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ... tests ...

    #[tokio::test]
    #[serial]
    async fn test_store_telemetry_logs() {
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
        store_telemetry_logs(logs.clone()).await;
        let stored = get_telemetry_logs().await;
        assert!(stored.len() >= 2);
    }

    // ============================================================================
    // Tests for event payload storage functions
    // ============================================================================

    #[tokio::test]
    #[serial]
    async fn test_store_and_get_event_payload() {
        let invocation_id = "test-invocation-store-get";
        let payload = r#"{"test": "data"}"#;

        store_event_payload(invocation_id, payload).await;
        let result = get_event_payload(invocation_id).await;

        assert!(result.is_some());
        assert_eq!(result.unwrap().as_str(), payload);
    }

    #[tokio::test]
    #[serial]
    async fn test_get_event_payload_not_found() {
        let result = get_event_payload("non-existent-invocation-id").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    #[serial]
    async fn test_take_event_payload_removes_entry() {
        let invocation_id = "test-invocation-take";
        let payload = r#"{"test": "data to take"}"#;

        store_event_payload(invocation_id, payload).await;

        // First take should return the payload
        let result1 = take_event_payload(invocation_id).await;
        assert!(result1.is_some());
        assert_eq!(result1.unwrap().as_str(), payload);

        // Second take should return None (already removed)
        let result2 = take_event_payload(invocation_id).await;
        assert_eq!(result2, None);

        // get should also return None
        let result3 = get_event_payload(invocation_id).await;
        assert_eq!(result3, None);
    }

    #[tokio::test]
    #[serial]
    async fn test_store_overwrites_existing_payload() {
        let invocation_id = "test-invocation-overwrite";
        let payload1 = r#"{"first": "payload"}"#;
        let payload2 = r#"{"second": "payload"}"#;

        store_event_payload(invocation_id, payload1).await;
        store_event_payload(invocation_id, payload2).await;

        let result = get_event_payload(invocation_id).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_str(), payload2);
    }

    // ============================================================================
    // Tests for invocation data storage functions
    // ============================================================================

    #[tokio::test]
    #[serial]
    async fn test_update_invocation_data_creates_new() {
        let invocation_id = "test-inv-data-create";

        update_invocation_data(invocation_id, |data| {
            data.start_time = 100.0;
        }).await;

        let result = get_invocation_data(invocation_id).await.expect("Should exist");
        assert_eq!(result.start_time, 100.0);
        // Check default values
        assert_eq!(result.duration, 0.0);
    }

    #[tokio::test]
    #[serial]
    async fn test_update_invocation_data_updates_existing_and_preserves() {
        let invocation_id = "test-inv-data-update";

        // First update: set start_time
        update_invocation_data(invocation_id, |data| {
            data.start_time = 100.0;
        }).await;

        // Second update: set duration, verifying start_time is preserved
        update_invocation_data(invocation_id, |data| {
            data.duration = 50.0;
        }).await;

        let result = get_invocation_data(invocation_id).await.expect("Should exist");
        assert_eq!(result.start_time, 100.0);
        assert_eq!(result.duration, 50.0);
    }

    // ============================================================================
    // Tests for invocation span ID storage functions
    // ============================================================================

    #[cfg(test)]
    async fn clear_invocation_span_ids() {
        // Clear both variants to ensure complete test isolation
        // This prevents cross-contamination between tests running with different configs
        INVOCATION_SPAN_IDS_RWLOCK.write().await.clear();
        INVOCATION_SPAN_IDS_MUTEX.lock().clear();
    }

    #[cfg(test)]
    async fn invocation_span_ids_len() -> usize {
        if crate::config::performance::get_config().use_tokio_rwlock {
            INVOCATION_SPAN_IDS_RWLOCK.read().await.len()
        } else {
            INVOCATION_SPAN_IDS_MUTEX.lock().len()
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_invocation_span_ids_max_capacity() {
        clear_invocation_span_ids().await;

        // Store 10 span IDs with small delays to ensure different timestamps
        for i in 0..10 {
            store_invocation_span_id(
                &format!("inv-{}", i),
                format!("trace-{}", i),
                format!("span-{}", i),
            ).await;
            // Small sleep to ensure distinct timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }

        // Verify all 10 are present
        assert_eq!(invocation_span_ids_len().await, 10);
        for i in 0..10 {
            assert!(
                get_invocation_span_id(&format!("inv-{}", i)).await.is_some(),
                "inv-{} should exist",
                i
            );
        }

        // Add an 11th item
        store_invocation_span_id("inv-10", "trace-10".to_string(), "span-10".to_string()).await;

        // Verify the map still has only 10 items
        assert_eq!(invocation_span_ids_len().await, 10);

        // Verify the oldest one (inv-0) was removed
        assert!(
            get_invocation_span_id("inv-0").await.is_none(),
            "inv-0 should have been evicted"
        );

        // Verify the newest one exists
        assert!(
            get_invocation_span_id("inv-10").await.is_some(),
            "inv-10 should exist"
        );

        // Verify items 1-9 still exist
        for i in 1..10 {
            assert!(
                get_invocation_span_id(&format!("inv-{}", i)).await.is_some(),
                "inv-{} should still exist",
                i
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_invocation_span_ids_update_existing_does_not_evict() {
        clear_invocation_span_ids().await;

        // Store 10 span IDs
        for i in 0..10 {
            store_invocation_span_id(
                &format!("inv-{}", i),
                format!("trace-{}", i),
                format!("span-{}", i),
            ).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }

        assert_eq!(invocation_span_ids_len().await, 10);

        // Update an existing key (should not evict anything)
        store_invocation_span_id(
            "inv-0",
            "trace-0-updated".to_string(),
            "span-0-updated".to_string(),
        ).await;

        // Verify still 10 items
        assert_eq!(invocation_span_ids_len().await, 10);

        // Verify the update took effect
        let updated = get_invocation_span_id("inv-0").await.expect("inv-0 should exist");
        assert_eq!(updated.trace_id, "trace-0-updated");
        assert_eq!(updated.span_id, "span-0-updated");

        // Verify all others still exist
        for i in 1..10 {
            assert!(
                get_invocation_span_id(&format!("inv-{}", i)).await.is_some(),
                "inv-{} should still exist",
                i
            );
        }
    }
}

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct StoredTrace {
    pub method: hyper::Method,
    pub path_and_query: String,
    pub headers: hyper::HeaderMap,
    pub body: Vec<u8>,
    pub invocation_ids: Vec<String>,
}

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
    #[serde(skip_deserializing)]
    pub trace_id: Option<String>,
    #[serde(skip_deserializing)]
    pub span_id: Option<String>,
}

pub fn store_runtime_done_notifier(sender: tokio::sync::oneshot::Sender<()>) {
    *RUNTIME_DONE_NOTIFIER.lock() = Some(sender);
}

pub fn take_runtime_done_notifier() -> Option<tokio::sync::oneshot::Sender<()>> {
    RUNTIME_DONE_NOTIFIER.lock().take()
}

static RUNTIME_DONE_NOTIFIER: Lazy<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
    Lazy::new(|| Mutex::new(None));

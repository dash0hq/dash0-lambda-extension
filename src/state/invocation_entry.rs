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

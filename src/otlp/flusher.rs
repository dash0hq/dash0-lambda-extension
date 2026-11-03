//! Bounds the telemetry the extension buffers across all signals (traces, OTLP
//! logs, telemetry logs, metrics).
//!
//! While an invocation runs, a background task flushes the buffer when it
//! reaches `DASH0_FLUSH_THRESHOLD` or when the last flush is
//! `DASH0_FLUSH_INTERVAL` old. Short invocations hit neither, so they keep a
//! single flush at invocation end. Incoming telemetry that would grow the
//! buffer past `DASH0_MAX_BUFFER_SIZE` is dropped.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::config::{flush_interval_ms, flush_threshold_bytes, max_buffer_bytes};

static INVOCATION_RUNNING: AtomicBool = AtomicBool::new(false);
static DROPPED_BYTES: AtomicUsize = AtomicUsize::new(0);
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);
static LAST_FLUSH: Lazy<Mutex<Instant>> = Lazy::new(|| Mutex::new(Instant::now()));

/// Bytes of telemetry buffered across all signals.
pub fn buffered_bytes() -> usize {
    crate::state::invocation_entry::buffered_bytes()
        + crate::state::invocation_data::buffered_bytes()
}

/// Call before buffering `incoming` bytes received from the function.
/// Returns false when the buffer is full and the data must be dropped.
pub fn admit(incoming: usize, signal: &str) -> bool {
    let buffered = buffered_bytes();
    let max = max_buffer_bytes();
    if buffered + incoming <= max {
        return true;
    }
    if DROPPED_BYTES.fetch_add(incoming, Ordering::Relaxed) == 0 {
        tracing::warn!(
            "[{}] Telemetry buffer full ({} of {} bytes); dropping {}",
            crate::log_prefix(),
            buffered,
            max,
            signal
        );
    }
    false
}

/// Call after buffering telemetry.
pub fn stored() {
    if INVOCATION_RUNNING.load(Ordering::Relaxed) && buffered_bytes() >= flush_threshold_bytes() {
        WAKE.notify_one();
    }
}

pub fn invocation_started() {
    *LAST_FLUSH.lock() = Instant::now();
    INVOCATION_RUNNING.store(true, Ordering::Relaxed);
    WAKE.notify_one();
}

/// Call at platform.runtimeDone. Stops early flushes, so none is in flight
/// when the execution environment freezes.
pub fn invocation_finished() {
    INVOCATION_RUNNING.store(false, Ordering::Relaxed);
    let dropped = DROPPED_BYTES.swap(0, Ordering::Relaxed);
    if dropped > 0 {
        tracing::warn!(
            "[{}] Dropped {} bytes of telemetry because the buffer was full (DASH0_MAX_BUFFER_SIZE)",
            crate::log_prefix(),
            dropped
        );
    }
}

pub fn spawn() {
    tokio::spawn(run());
}

async fn run() {
    let interval = Duration::from_millis(flush_interval_ms());
    loop {
        if !INVOCATION_RUNNING.load(Ordering::Relaxed) || interval.is_zero() {
            WAKE.notified().await;
        } else {
            let deadline = *LAST_FLUSH.lock() + interval;
            tokio::select! {
                _ = WAKE.notified() => {}
                _ = tokio::time::sleep_until(deadline) => {}
            }
        }
        if !INVOCATION_RUNNING.load(Ordering::Relaxed) {
            continue;
        }
        let interval_due = !interval.is_zero() && *LAST_FLUSH.lock() + interval <= Instant::now();
        if interval_due || buffered_bytes() >= flush_threshold_bytes() {
            *LAST_FLUSH.lock() = Instant::now();
            crate::otlp::exporter::flush_while_running().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::invocation_data::{self, StoredMetric, StoredTrace};
    use crate::state::invocation_entry;
    use serial_test::serial;

    const INV: &str = "inv-flusher";

    fn reset() {
        invocation_finished();
        let _ = invocation_entry::take_all_traces();
        let _ = invocation_entry::take_all_telemetry_logs(None);
        let _ = invocation_data::take_logs();
        let _ = invocation_data::take_metrics();
        for var in [
            "DASH0_MAX_BUFFER_SIZE",
            "DASH0_FLUSH_THRESHOLD",
            "DASH0_FLUSH_INTERVAL",
        ] {
            std::env::remove_var(var);
        }
    }

    fn store_trace(bytes: usize, contains_handler_span: bool) {
        invocation_entry::store_trace_by_id(
            INV,
            StoredTrace {
                method: hyper::Method::POST,
                path_and_query: "/v1/traces".to_string(),
                headers: hyper::HeaderMap::new(),
                body: vec![0; bytes],
                invocation_ids: vec![INV.to_string()],
                contains_handler_span,
            },
        );
    }

    /// Polls `cond` for up to 2 s of real time.
    async fn eventually(cond: impl Fn() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[test]
    #[serial]
    fn admit_counts_all_signals_against_the_cap() {
        reset();
        std::env::set_var("DASH0_MAX_BUFFER_SIZE", "10"); // 10 KiB
        store_trace(4 * 1024, false);
        invocation_data::store_metric(StoredMetric {
            method: hyper::Method::POST,
            path_and_query: "/v1/metrics".to_string(),
            headers: hyper::HeaderMap::new(),
            body: vec![0; 4 * 1024],
        });
        assert_eq!(buffered_bytes(), 8 * 1024);

        assert!(admit(2 * 1024, "traces"), "exactly at the cap is admitted");
        assert!(!admit(2 * 1024 + 1, "traces"), "one byte over is dropped");
        reset();
    }

    #[tokio::test]
    #[serial]
    async fn flushes_child_spans_at_threshold_and_keeps_handler_span() {
        reset();
        std::env::set_var("DASH0_FLUSH_THRESHOLD", "4"); // 4 KiB
        std::env::set_var("DASH0_FLUSH_INTERVAL", "0"); // size trigger only
        tokio::spawn(run());
        invocation_started();

        store_trace(1024, true);
        store_trace(1024, false);
        stored();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            buffered_bytes(),
            2048,
            "below the threshold nothing is sent"
        );

        store_trace(3 * 1024, false);
        stored();
        assert!(
            eventually(|| buffered_bytes() == 1024).await,
            "child spans flushed, only the handler span request left"
        );
        let left = invocation_entry::take_all_traces();
        assert_eq!(left.len(), 1);
        assert!(left[0].contains_handler_span);
        reset();
    }

    #[tokio::test]
    #[serial]
    async fn flushes_on_interval_while_invocation_runs() {
        reset();
        std::env::set_var("DASH0_FLUSH_INTERVAL", "300");
        tokio::spawn(run());
        invocation_started();
        let start = Instant::now();
        store_trace(100, false);
        stored();

        assert!(eventually(|| buffered_bytes() == 0).await);
        assert!(
            start.elapsed() >= Duration::from_millis(250),
            "flushed after {:?}, before the interval",
            start.elapsed()
        );
        reset();
    }

    #[tokio::test]
    #[serial]
    async fn does_not_flush_between_invocations() {
        reset();
        std::env::set_var("DASH0_FLUSH_THRESHOLD", "1");
        std::env::set_var("DASH0_FLUSH_INTERVAL", "50");
        tokio::spawn(run());
        invocation_started();
        invocation_finished();

        store_trace(4 * 1024, false);
        stored();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(buffered_bytes(), 4 * 1024);
        reset();
    }
}

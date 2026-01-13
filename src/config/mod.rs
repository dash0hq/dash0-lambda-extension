/// Configuration modules for the Dash0 Lambda extension

pub mod general;
pub mod performance;

// Re-export general config functions for backward compatibility
pub use general::{
    is_auto_instrumented_disabled,
    is_send_on_invocation_end,
    max_event_payload_size,
    request_retries,
    request_timeout_ms,
};

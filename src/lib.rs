// Library exports for benchmarks and tests
//
// This file makes internal modules accessible to benchmarks without
// exposing them in the binary's public API.

// Declare modules
mod env;
pub(crate) mod route;
mod sandbox;
mod stats;

// Public exports for benchmarks
pub mod backend_send;
pub mod config;
pub mod store;
pub mod util;

// Re-export commonly used types for convenience
pub use store::{StoredTrace, PayloadValue};

// Constants needed by modules
pub const EXTENSION_NAME: &str = "lrap";
pub const DEFAULT_PROXY_PORT: u16 = 9009;
pub static LAMBDA_RUNTIME_API_VERSION: &str = "2018-06-01";

// Helper functions needed by modules
#[inline]
pub fn log_prefix() -> &'static str {
    "DASH0"
}

#[inline]
pub fn log_prefix_with(suffix: &str) -> String {
    format!("DASH0:{}", suffix)
}

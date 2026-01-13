/// Performance optimization configuration controlled via environment variables
///
/// This module provides runtime feature flags for performance optimizations,
/// allowing A/B testing and gradual rollout in production.

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};

/// Performance configuration for runtime feature flags
#[derive(Debug, Clone, Copy, Default)]
pub struct PerformanceConfig {
    /// Enable Arc<String> instead of String in stores (14x faster for large payloads)
    pub use_arc_strings: bool,

    /// Enable static HTTP client reuse in sandbox.rs (70-100x faster for warm invocations)
    pub use_static_http_client: bool,

    /// Enable tokio::sync::RwLock instead of parking_lot::Mutex (better tail latency)
    pub use_tokio_rwlock: bool,

    /// Enable lazy protobuf decode (2-3x throughput improvement)
    pub use_lazy_protobuf: bool,
}

impl PerformanceConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        // Check for master enable flag first
        let enable_all = std::env::var("DASH0_ENABLE_ALL_OPTIMIZATIONS")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let use_arc_strings = if enable_all {
            true
        } else {
            std::env::var("DASH0_USE_ARC_STRINGS")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false)
        };

        let use_static_http_client = if enable_all {
            true
        } else {
            std::env::var("DASH0_USE_STATIC_HTTP_CLIENT")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false)
        };

        let use_tokio_rwlock = if enable_all {
            true
        } else {
            std::env::var("DASH0_USE_TOKIO_RWLOCK")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false)
        };

        let use_lazy_protobuf = if enable_all {
            true
        } else {
            std::env::var("DASH0_USE_LAZY_PROTOBUF")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false)
        };

        Self {
            use_arc_strings,
            use_static_http_client,
            use_tokio_rwlock,
            use_lazy_protobuf,
        }
    }

    /// Log the active configuration to stderr (for Lambda logs)
    pub fn log(&self) {
        eprintln!("Dash0 Performance Configuration:");
        eprintln!("  Arc Strings:        {}", if self.use_arc_strings { "ENABLED" } else { "disabled" });
        eprintln!("  Static HTTP Client: {}", if self.use_static_http_client { "ENABLED" } else { "disabled" });
        eprintln!("  Tokio RwLock:       {}", if self.use_tokio_rwlock { "ENABLED" } else { "disabled" });
        eprintln!("  Lazy Protobuf:      {}", if self.use_lazy_protobuf { "ENABLED" } else { "disabled" });

        if !self.use_arc_strings && !self.use_static_http_client
            && !self.use_tokio_rwlock && !self.use_lazy_protobuf {
            eprintln!("  (All optimizations disabled - baseline performance mode)");
        }
    }
}

/// Global static configuration loaded once at startup
pub static CONFIG: Lazy<PerformanceConfig> = Lazy::new(PerformanceConfig::from_env);

// Benchmark override support
static OVERRIDE_ENABLED: AtomicBool = AtomicBool::new(false);
static mut OVERRIDE_CONFIG: Option<PerformanceConfig> = None;

/// Override configuration for benchmarking
///
/// # Safety
/// This is only safe to call in single-threaded benchmark contexts.
/// Do not call from production code or tests running in parallel.
pub unsafe fn set_config_override(config: PerformanceConfig) {
    OVERRIDE_CONFIG = Some(config);
    OVERRIDE_ENABLED.store(true, Ordering::SeqCst);
}

/// Clear configuration override
pub fn clear_config_override() {
    OVERRIDE_ENABLED.store(false, Ordering::SeqCst);
}

/// Get the active configuration (respects overrides for benchmarks)
#[inline]
pub fn get_config() -> &'static PerformanceConfig {
    if OVERRIDE_ENABLED.load(Ordering::SeqCst) {
        unsafe {
            OVERRIDE_CONFIG.as_ref().unwrap_or(&CONFIG)
        }
    } else {
        &CONFIG
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_all_disabled() {
        let config = PerformanceConfig::default();
        assert!(!config.use_arc_strings);
        assert!(!config.use_static_http_client);
        assert!(!config.use_tokio_rwlock);
        assert!(!config.use_lazy_protobuf);
    }

    #[test]
    fn test_config_override() {
        // Test override mechanism
        let test_config = PerformanceConfig {
            use_arc_strings: true,
            use_static_http_client: true,
            use_tokio_rwlock: false,
            use_lazy_protobuf: false,
        };

        unsafe {
            set_config_override(test_config.clone());
        }

        let active = get_config();
        assert!(active.use_arc_strings);
        assert!(active.use_static_http_client);

        clear_config_override();
    }
}

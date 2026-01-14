use once_cell::sync::OnceCell;

/// Sandbox's Runtime API endpoint
static LAMBDA_RUNTIME_API: OnceCell<String> = OnceCell::new();

/// Lambda Runtime API Proxy (LRAP), this endpoint
static LRAP_API: OnceCell<String> = OnceCell::new();

/// Get the extension port from environment variable or use default
pub fn extension_port() -> u16 {
    use std::env::var;

    var("DASH0_LISTENER_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(crate::DEFAULT_PROXY_PORT)
}

/// Latch in the API endpoints defined in ENV variables
///
#[allow(dead_code)]
pub fn latch_runtime_env() {
    use std::env::var;

    let aws_lambda_runtime_api =
        match var("LRAP_RUNTIME_API_ENDPOINT").or_else(|_| var("AWS_LAMBDA_RUNTIME_API")) {
            Ok(v) => v,
            Err(_) => panic!("LRAP_RUNTIME_API_ENDPOINT or AWS_LAMBDA_RUNTIME_API not found"),
        };

    // Latch in the ORIGIN we should proxy to the application
    if let Err(_) = LAMBDA_RUNTIME_API.set(aws_lambda_runtime_api.clone()) {
        tracing::warn!(
            "[{}] AWS_LAMBDA_RUNTIME_API was already set, skipping",
            crate::log_prefix()
        );
    }

    let lrap_api = format!("0.0.0.0:{}", extension_port());

    if let Err(_) = LRAP_API.set(lrap_api.clone()) {
        tracing::warn!(
            "[{}] LRAP_API was already set, skipping",
            crate::log_prefix()
        );
    }
}

/// Gets the original AWS_LAMBDA_RUNTIME_API.
///
#[allow(dead_code)]
pub fn sandbox_runtime_api() -> &'static str {
    match LAMBDA_RUNTIME_API.get() {
        Some(val) => val,
        None => {
            latch_runtime_env();
            LAMBDA_RUNTIME_API.get().unwrap_or_else(|| {
                tracing::error!(
                    "[{}] Failed to initialize AWS_LAMBDA_RUNTIME_API",
                    crate::log_prefix()
                );
                panic!(
                    "[{}] Cannot proceed without runtime API configuration",
                    crate::log_prefix()
                );
            })
        }
    }
}

/// Gets the new LRAP_API.
///
pub fn lrap_api() -> &'static str {
    match LRAP_API.get() {
        Some(val) => val,
        None => {
            latch_runtime_env();
            LRAP_API.get().unwrap_or_else(|| {
                tracing::error!(
                    "[{}] Failed to initialize LRAP_API host:port",
                    crate::log_prefix()
                );
                panic!(
                    "[{}] Cannot proceed without proxy listener configuration",
                    crate::log_prefix()
                );
            })
        }
    }
}

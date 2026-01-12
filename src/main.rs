#[allow(unused_imports)]
use std::{
    convert::Infallible,
    io::{Read, Write},
    net::SocketAddr,
    process::Stdio,
    sync::Arc,
};

#[allow(unused_imports)]
use hyper::{Body, Request, Response, Server};

use tokio::{self};
use tracing_subscriber::EnvFilter;

pub mod config;
/// ENV references to API endpoints (host:port)
mod env;

/// Routes for Lambda Runtime API
mod route;

/// Common utilities
pub mod util;

pub mod backend_send;
pub(crate) mod sandbox;
pub(crate) mod stats;
pub(crate) mod store;

/// Name to register with the Lambda Extension API.
///
/// NOTE: this must be the same as the
/// entrypoint script destination in the Lambda layer (eg, **extensions/lrap**)
pub const EXTENSION_NAME: &str = "lrap";

/// Default port to listen on, overriden by LRAP_LISTENER_PORT environment variable
///
/// NOTE: this must be the same port as listed in **opt/wrapper** script that launches
/// the Application runtime with the modified `AWS_LAMBDA_RUNTIME_API` env variable.
pub const DEFAULT_PROXY_PORT: u16 = 9009;

pub static LAMBDA_RUNTIME_API_VERSION: &str = "2018-06-01";

/// Returns the log prefix for standard log messages
#[inline]
pub fn log_prefix() -> &'static str {
    "DASH0"
}

/// Returns the log prefix with a suffix for specialized log messages
#[inline]
pub fn log_prefix_with(suffix: &str) -> String {
    format!("DASH0:{}", suffix)
}

/// Implement the Runtime API Proxy for Lambda:
///
/// 1. create a hyper server on the LRAP endpoint
///
/// 2. create a Tower service for the Lambda Runtime API to serve HTTP requests
///
/// 3. register as an Extension, allowing Application runtime to begin initializing
///
/// 4. request `next` event from Extension API, fulfilling lifecycle contract
///   
///
#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_env("OTEL_EXTENSION_LOG_LEVEL")
        .unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_current_span(true)
        .with_level(true)
        .with_target(true)
        .flatten_event(true)
        .init();
    stats::init_start();

    let exe_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "<unknown>".to_string());
    tracing::info!("[{}] start; path={}", crate::log_prefix(), exe_path);

    tracing::info!(
        "[{}] commandline arguments: {}",
        crate::log_prefix(),
        std::env::args()
            .map(|v| format!("\"{}\"", v))
            .collect::<Vec<String>>()
            .join(", ")
    );

    env::latch_runtime_env();

    let addr: SocketAddr = match env::lrap_api().parse() {
        Ok(addr) => addr,
        Err(e) => {
            tracing::error!(
                "[{}] Invalid IP specification from Lambda Runtime API endpoint: {}",
                crate::log_prefix(),
                e
            );
            panic!(
                "[{}] Cannot start without valid listener address",
                crate::log_prefix()
            );
        }
    };
    tracing::info!("[{}] listening on {}", crate::log_prefix(), addr);

    // bind the server to the Lambda Runtime API Router service
    let server = Server::bind(&addr).serve(route::make_route().into_service());

    // launch the Proxy server task
    let server_join_handle = tokio::spawn(server);

    // Initialize the extension and continually get next extension event.
    tokio::task::spawn(async {
        sandbox::extension::register().await;
        sandbox::extension::register_telemetry().await;
        // Lambda Application runtime will start once our extension is registered
        stats::app_start();

        loop {
            // Lambda Extension API requires we wait for next extension event
            sandbox::extension::get_next().await;
        }
    });

    match server_join_handle.await {
        Ok(Ok(_)) => {
            // Server shut down cleanly (should never happen)
            tracing::info!("[{}] Server shut down cleanly", crate::log_prefix());
        }
        Ok(Err(e)) => {
            tracing::error!("[{}] Hyper server error: {}", crate::log_prefix(), e);
        }
        Err(e) => {
            tracing::error!(
                "[{}] Failed to join server task: {}",
                crate::log_prefix(),
                e
            );
        }
    }
}

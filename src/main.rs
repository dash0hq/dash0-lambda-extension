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

use crate::otlp::masking::init_masking_rules;
use tokio::{self};
use tracing_subscriber::EnvFilter;

pub mod config;
pub mod extension;
pub mod route;
pub mod state;

pub mod util;

pub mod otlp;
pub(crate) mod stats;

/// Name to register with the Lambda Extension API.
/// NOTE: this must be the same as the
/// entrypoint script destination in the Lambda layer (eg, **extensions/dash0**)
pub const EXTENSION_NAME: &str = "dash0";

/// Default port to listen on, overriden by DASH0_LISTENER_PORT environment variable
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

/// Four initialization tasks:
///
/// 1. create a hyper server
/// 2. create a Tower service for the Lambda Runtime API to serve HTTP requests
/// 3. register as an Extension, allowing Application runtime to begin initializing
/// 4. request `next` event from Extension API, fulfilling lifecycle contract
///
#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_new(&config::extension_log_level())
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

    config::endpoints::latch_runtime_env();

    init_masking_rules();

    let addr: SocketAddr = match config::endpoints::dash0_api().parse() {
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
        extension::register::register().await;
        extension::register::register_telemetry().await;
        // Lambda Application runtime will start once our extension is registered
        stats::app_start();

        loop {
            // Lambda Extension API requires we wait for next extension event
            extension::events::get_next().await;
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

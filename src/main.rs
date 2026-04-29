use std::net::SocketAddr;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

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
///
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
/// 2. dispatch incoming HTTP requests through the route table to handlers
/// 3. register as an Extension, allowing Application runtime to begin initializing
/// 4. request `next` event from Extension API, fulfilling lifecycle contract
///
#[tokio::main]
async fn main() {
    // Both `ring` and `aws-lc-rs` are pulled into the dependency tree (tonic enables
    // aws-lc-rs transitively), so rustls cannot auto-select a CryptoProvider. Pick ring
    // explicitly before anything builds a TLS connector.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default rustls crypto provider");

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
    config::token::init_dash0_token().await;

    init_masking_rules();
    route::init();

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

    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("[{}] Failed to bind listener: {}", crate::log_prefix(), e);
            panic!(
                "[{}] Cannot start without bound listener",
                crate::log_prefix()
            );
        }
    };

    let server_join_handle = tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::error!("[{}] accept() failed: {}", crate::log_prefix(), e);
                    continue;
                }
            };
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                if let Err(e) = http1::Builder::new()
                    .serve_connection(io, service_fn(route::dispatch))
                    .await
                {
                    tracing::debug!("[{}] connection error: {}", crate::log_prefix(), e);
                }
            });
        }
    });

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

    if let Err(e) = server_join_handle.await {
        tracing::error!(
            "[{}] Failed to join server task: {}",
            crate::log_prefix(),
            e
        );
    }
}

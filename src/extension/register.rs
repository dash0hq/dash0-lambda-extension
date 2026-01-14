use hyper::Body;
use once_cell::sync::OnceCell;

use crate::config::endpoints;

/// Canonical Lambda Extensions API version
const EXTENSION_API_VERSION: &str = "2020-01-01";

static LAMBDA_EXTENSION_IDENTIFIER: OnceCell<String> = OnceCell::new();

pub fn extension_id() -> &'static String {
    match LAMBDA_EXTENSION_IDENTIFIER.get() {
        Some(id) => id,
        None => {
            tracing::error!(
                "[{}] Lambda Extension Identifier not set - extension not registered",
                crate::log_prefix_with("Extension")
            );
            panic!(
                "[{}] Extension must be registered before use",
                crate::log_prefix_with("Extension")
            );
        }
    }
}

fn find_extension_name() -> String {
    crate::EXTENSION_NAME.to_owned()
}

fn make_uri(path: &str) -> hyper::Uri {
    match hyper::Uri::builder()
        .scheme("http")
        .authority(endpoints::sandbox_runtime_api())
        .path_and_query(path)
        .build()
    {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!(
                "[{}] Error building Lambda Extensions API endpoint URL: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            panic!(
                "[{}] Failed to build Extensions API URI - severe misconfiguration: {}",
                crate::log_prefix_with("Extension"),
                e
            );
        }
    }
}

/// Register the extension with the Lambda Extensions API
pub async fn register() {
    let uri = make_uri(&format!("/{}/extension/register", EXTENSION_API_VERSION));

    let body = Body::from(r#"{"events":["INVOKE","SHUTDOWN"]}"#);
    let mut request = match hyper::Request::builder().method("POST").uri(uri).body(body) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!(
                "[{}] Cannot create Lambda Extensions API request: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            panic!(
                "[{}] Failed to create extension registration request: {}",
                crate::log_prefix_with("Extension"),
                e
            );
        }
    };

    // Set Lambda Extension Name header
    match find_extension_name().try_into() {
        Ok(header_value) => {
            request
                .headers_mut()
                .append("Lambda-Extension-Name", header_value);
        }
        Err(e) => {
            tracing::error!(
                "[{}] Invalid extension name: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            panic!(
                "[{}] Cannot register with invalid extension name: {}",
                crate::log_prefix_with("Extension"),
                e
            );
        }
    }

    let response = match hyper::Client::new().request(request).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(
                "[{}] Cannot send Lambda Extensions API request to register: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            panic!(
                "[{}] Failed to register extension: {}",
                crate::log_prefix_with("Extension"),
                e
            );
        }
    };

    let extension_identifier = match response.headers().get("lambda-extension-identifier") {
        Some(header) => match header.to_str() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(
                    "[{}] Invalid extension identifier header: {}",
                    crate::log_prefix_with("Extension"),
                    e
                );
                panic!(
                    "[{}] Cannot parse extension identifier: {}",
                    crate::log_prefix_with("Extension"),
                    e
                );
            }
        },
        None => {
            tracing::error!(
                "[{}] Lambda Extensions API response missing 'lambda-extension-identifier' header",
                crate::log_prefix_with("Extension")
            );
            panic!(
                "[{}] Extension registration failed - missing identifier header",
                crate::log_prefix_with("Extension")
            );
        }
    };

    if let Err(e) = LAMBDA_EXTENSION_IDENTIFIER.set(extension_identifier.to_owned()) {
        tracing::warn!(
            "[{}] Extension identifier already set: {:?}",
            crate::log_prefix_with("Extension"),
            e
        );
    }
}

pub async fn register_telemetry() {
    let uri = make_uri("/2022-07-01/telemetry");
    let destination = format!(
        "http://sandbox.localdomain:{}/v1/telemetry",
        crate::DEFAULT_PROXY_PORT
    );
    let payload = format!(
        r#"{{"schemaVersion":"2022-07-01","destination":{{"protocol":"HTTP","URI":"{}"}},"types":["platform","function"]}}"#,
        destination
    );
    // print the payload
    tracing::trace!(
        "[{}] Registering telemetry with payload={}",
        crate::log_prefix_with("Extension"),
        payload
    );

    let mut request = match hyper::Request::builder()
        .method("PUT")
        .uri(uri.clone())
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload))
    {
        Ok(req) => req,
        Err(e) => {
            tracing::error!(
                "[{}] Cannot create Lambda Telemetry API request: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            return;
        }
    };

    match extension_id().try_into() {
        Ok(header_value) => {
            request
                .headers_mut()
                .insert("Lambda-Extension-Identifier", header_value);
        }
        Err(e) => {
            tracing::error!(
                "[{}] Invalid extension identifier for telemetry registration: {}",
                crate::log_prefix_with("Extension"),
                e
            );
            return;
        }
    }

    match hyper::Client::new().request(request).await {
        Ok(response) => {
            let status = response.status();
            let body_bytes = match hyper::body::to_bytes(response.into_body()).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    tracing::error!(
                        "[{}] Failed to read telemetry registration body: {}",
                        crate::log_prefix_with("Extension"),
                        err
                    );
                    return;
                }
            };
            tracing::info!(
                "[{}] Telemetry register uri={} status={} body={}",
                crate::log_prefix_with("Extension"),
                uri,
                status,
                String::from_utf8_lossy(&body_bytes)
            );
        }
        Err(err) => {
            tracing::error!(
                "[{}] Error registering telemetry destination (uri={}): {}",
                crate::log_prefix_with("Extension"),
                uri,
                err
            );
        }
    }
}

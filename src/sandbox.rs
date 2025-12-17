//
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: MIT-0
//

//! Interact with the Lambda Runtime API, the service managing this sandbox
//!
//! Includes helpers for sending request for `next` and posting back responses.
//!

use aws_config::BehaviorVersion;
use aws_sdk_sts::Client as StsClient;
use hyper::{Body, Error, HeaderMap, Request, Response};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;

pub async fn next(headers: &HeaderMap, path: &str) -> Result<(Arc<String>, Response<Body>), Error> {
    let uri = hyper::Uri::builder()
        .scheme("http")
        .authority(crate::env::sandbox_runtime_api())
        .path_and_query(path)
        .build()
        .expect("[LRAP] Error building Sandbox Lambda Runtime API endpoint URL");

    let mut req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("Cannot create Sandbox Lambda Runtime API request");

    *req.headers_mut() = headers.clone();

    let response = hyper::Client::new().request(req).await?;

    match response.headers().get("lambda-runtime-aws-request-id") {
        Some(id) => {
            let id = id.to_str().expect("Error parsing Lambda Runtime API request ID");
            Ok((Arc::new(id.to_string()), response))
        },
        // PANIC OK: when Lambda Runtime API does not meet its API contract, we kill the application
        _ => panic!("[LRAP] Sandbox Lambda Runtime API response missing 'lambda-runtime-aws-request-id' header in Lambda Runtime API GET:next response") 
    }
}

/// Send a request through a {hyper::Client}
pub async fn send_request(request: Request<Body>) -> Result<Response<Body>, Error> {
    hyper::Client::new().request(request).await
}

#[allow(dead_code)]
pub async fn create_invoke_result_request(id: &str, body: Body) -> Result<Request<Body>, Error> {
    let uri = hyper::Uri::builder()
        .scheme("http")
        .authority(crate::env::sandbox_runtime_api())
        .path_and_query(format!(
            "/{}/runtime/invocation/{}/response",
            crate::LAMBDA_RUNTIME_API_VERSION,
            id
        ))
        .build()
        .expect("[LRAP] Error building Sandbox Lambda Runtime API endpoint URL");

    Ok(hyper::Request::builder()
        .method("POST")
        .uri(uri)
        .body(body)
        .expect("Cannot create Sandbox Lambda Runtime API request"))
}

/// Lambda Extensions API
///
/// Interact with the Lambda sandbox as a Lambda Extension
///
#[allow(dead_code)]
pub mod extension {
    use crate::DEFAULT_PROXY_PORT;
    use hyper::Body;
    use once_cell::sync::OnceCell;
    use std::time::{Duration, Instant};
    /// Cannonical Lambda Extensions API version
    ///
    /// Documentation: https://docs.aws.amazon.com/lambda/latest/dg/runtimes-extensions-api.html
    ///
    const EXTENSION_API_VERSION: &str = "2020-01-01";
    static LAMBDA_EXTENSION_IDENTIFIER: OnceCell<String> = OnceCell::new();

    fn find_extension_name() -> String {
        crate::EXTENSION_NAME.to_owned()
    }

    pub(super) fn extension_id() -> &'static String {
        LAMBDA_EXTENSION_IDENTIFIER
            .get()
            .expect("[LRAP:Extension] Lambda Extension Identifier not set!")
    }

    fn make_uri(path: &str) -> hyper::Uri {
        hyper::Uri::builder()
            .scheme("http")
            .authority(crate::env::sandbox_runtime_api())
            .path_and_query(format!("/{}/extension{}", EXTENSION_API_VERSION, path))
            .build()
            .expect("[LRAP:Extension] Error building Lambda Extensions API endpoint URL")
    }

    /// Register the extension with the Lambda Extensions API
    pub async fn register() {
        let uri = make_uri("/register");

        let body = hyper::Body::from(r#"{"events":["INVOKE","SHUTDOWN"]}"#);
        let mut request = hyper::Request::builder()
            .method("POST")
            .uri(uri)
            .body(body)
            .expect("[LRAP:Extension] Cannot create Lambda Extensions API request");

        // Set Lambda Extension Name header
        request.headers_mut().append(
            "Lambda-Extension-Name",
            find_extension_name().try_into().unwrap(),
        );

        let response = super::send_request(request)
            .await
            .expect("[LRAP:Extension] Cannot send Lambda Extensions API request to register");

        let extension_identifier = response
            .headers()
            .get("lambda-extension-identifier")
            .expect("[LRAP:Extension] Lambda Extensions API response missing 'lambda-extension-identifier' header in Lambda Extensions API POST:register response")
            .to_str()
            .unwrap();

        LAMBDA_EXTENSION_IDENTIFIER
            .set(extension_identifier.to_owned())
            .expect("[LRAP:Extension] Error setting Lambda Extensions API request ID");
    }

    /// Get next event from the Lambda Extensions API
    ///
    pub async fn get_next() {
        let uri = make_uri("/event/next");

        let mut request = hyper::Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("[LRAP:Extension] Cannot create Lambda Extensions API request");

        request.headers_mut().insert(
            "Lambda-Extension-Identifier",
            extension_id().try_into().unwrap(),
        );

        let start = Instant::now();
        match super::send_request(request).await {
            Ok(response) => {
                let status = response.status();
                let body_bytes = match hyper::body::to_bytes(response.into_body()).await {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        tracing::error!(
                            "[LRAP:Extension] Failed to read extension event body: {}",
                            err
                        );
                        return;
                    }
                };

                tracing::info!(
                    "[LRAP:Extension] Event status={} payload={} latency={} ms",
                    status,
                    String::from_utf8_lossy(&body_bytes),
                    start.elapsed().as_millis()
                );

                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    if json
                        .get("eventType")
                        .and_then(|v| v.as_str())
                        .map(|t| t == "INVOKE")
                        == Some(true)
                    {
                        if let Some(arn) = json.get("invokedFunctionArn").and_then(|v| v.as_str()) {
                            super::store_function_arn(arn);
                        }
                    }

                    let event_type = json.get("eventType").and_then(|v| v.as_str());
                    let shutdown_reason = json
                        .get("shutdownReason")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_lowercase());

                    let should_flush = (matches!(event_type, Some("INVOKE"))
                        && !crate::config::is_send_on_invocation_end())
                        || (matches!(event_type, Some("SHUTDOWN"))
                            && shutdown_reason.as_deref() == Some("spindown"));

                    if should_flush {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        let is_invocation_end = matches!(event_type, Some("SHUTDOWN"));
                        crate::backend_send::flush_traces(is_invocation_end).await;
                        crate::backend_send::flush_logs(is_invocation_end).await;
                    }

                    if matches!(event_type, Some("INVOKE"))
                        && crate::config::is_send_on_invocation_end()
                    {
                        // Block execution until platform.runtimeDone is received
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        crate::store::store_runtime_done_notifier(tx);

                        tracing::info!("[LRAP:Extension] Waiting for platform.runtimeDone");

                        // Wait for the signal with a timeout to prevent indefinite blocking
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(900), // 15 minute timeout (max Lambda duration)
                            rx,
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                tracing::info!(
                                    "[LRAP:Extension] Received platform.runtimeDone signal"
                                );
                                crate::backend_send::flush_traces(true).await;
                                crate::backend_send::flush_logs(true).await;
                            }
                            Ok(Err(_)) => {
                                tracing::warn!(
                                    "[LRAP:Extension] platform.runtimeDone channel closed"
                                );
                            }
                            Err(_) => {
                                tracing::error!(
                                    "[LRAP:Extension] Timeout waiting for platform.runtimeDone"
                                );
                            }
                        }
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    "[LRAP:Extension] Error fetching next extension event: {}",
                    err
                );
            }
        }
    }

    pub async fn register_telemetry() {
        let uri = make_telemetry_uri();
        let destination = format!(
            "http://sandbox.localdomain:{}/v1/telemetry",
            DEFAULT_PROXY_PORT
        );
        let payload = format!(
            r#"{{"schemaVersion":"2022-07-01","destination":{{"protocol":"HTTP","URI":"{}"}},"types":["platform","function"]}}"#,
            destination
        );
        // print the payload
        tracing::trace!(
            "[LRAP:Extension] Registering telemetry with payload={}",
            payload
        );

        let mut request = hyper::Request::builder()
            .method("PUT")
            .uri(uri.clone())
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(hyper::Body::from(payload))
            .expect("[LRAP:Extension] Cannot create Lambda Telemetry API request");

        request.headers_mut().insert(
            "Lambda-Extension-Identifier",
            extension_id().try_into().unwrap(),
        );

        match super::send_request(request).await {
            Ok(response) => {
                let status = response.status();
                let body_bytes = match hyper::body::to_bytes(response.into_body()).await {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        tracing::error!(
                            "[LRAP:Extension] Failed to read telemetry registration body: {}",
                            err
                        );
                        return;
                    }
                };
                tracing::info!(
                    "[LRAP:Extension] Telemetry register uri={} status={} body={}",
                    uri,
                    status,
                    String::from_utf8_lossy(&body_bytes)
                );
            }
            Err(err) => {
                tracing::error!(
                    "[LRAP:Extension] Error registering telemetry destination (uri={}): {}",
                    uri,
                    err
                );
            }
        }
    }

    fn make_telemetry_uri() -> hyper::Uri {
        hyper::Uri::builder()
            .scheme("http")
            .authority(crate::env::sandbox_runtime_api())
            .path_and_query("/2022-07-01/telemetry")
            .build()
            .expect("[LRAP:Extension] Error building Lambda Telemetry API endpoint URL")
    }
}

static FUNCTION_ARN: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static ACCOUNT_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

pub fn store_function_arn(arn: &str) {
    let mut guard = FUNCTION_ARN.lock();
    if guard.is_none() {
        *guard = Some(arn.to_string());
        if let Some(account) = parse_account_id_from_arn(arn) {
            let mut acct_guard = ACCOUNT_ID.lock();
            if acct_guard.is_none() {
                *acct_guard = Some(account);
            }
        }
    }
}

pub fn get_function_arn() -> Option<String> {
    let mut guard = FUNCTION_ARN.lock();
    if guard.as_ref().map(|arn| !arn.is_empty()).unwrap_or(false) {
        return guard.clone();
    }

    let account_id = get_account_id().filter(|id| !id.is_empty());
    let region = std::env::var("AWS_REGION").ok().filter(|r| !r.is_empty());
    let function_name = std::env::var("AWS_LAMBDA_FUNCTION_NAME")
        .ok()
        .filter(|name| !name.is_empty());

    match (account_id, region, function_name) {
        (Some(account_id), Some(region), Some(function_name)) => {
            let arn = format!(
                "arn:aws:lambda:{}:{}:function:{}",
                region, account_id, function_name
            );
            *guard = Some(arn.clone());
            Some(arn)
        }
        _ => None,
    }
}

pub fn get_account_id() -> Option<String> {
    ACCOUNT_ID.lock().clone()
}

pub async fn fetch_and_cache_account_id() -> Option<String> {
    let start = Instant::now();
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let client = StsClient::new(&config);
    match client.get_caller_identity().send().await {
        Ok(output) => {
            if let Some(account) = output.account() {
                let mut guard = ACCOUNT_ID.lock();
                if guard.as_ref().map(|id| !id.is_empty()).unwrap_or(false) {
                    return guard.clone();
                }
                let account = account.to_string();
                *guard = Some(account.clone());
                tracing::info!(
                    "[LRAP] Retrieved account ID={} from STS GetCallerIdentity in {} ms",
                    account,
                    start.elapsed().as_millis()
                );
                Some(account)
            } else {
                tracing::error!("[LRAP] STS GetCallerIdentity did not return an account ID");
                None
            }
        }
        Err(err) => {
            tracing::error!("[LRAP] Failed calling STS GetCallerIdentity: {}", err);
            None
        }
    }
}

fn parse_account_id_from_arn(arn: &str) -> Option<String> {
    // arn:partition:service:region:account-id:resource
    let parts: Vec<&str> = arn.split(':').collect();
    if parts.len() >= 5 {
        Some(parts[4].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{get_function_arn, store_function_arn, ACCOUNT_ID, FUNCTION_ARN};
    use serial_test::serial;
    use std::env;

    fn reset_state() {
        FUNCTION_ARN.lock().take();
        ACCOUNT_ID.lock().take();
        env::remove_var("AWS_REGION");
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
    }

    #[test]
    #[serial]
    fn returns_cached_function_arn_when_set() {
        reset_state();
        let cached = "arn:aws:lambda:us-east-1:123456789012:function:cached-func";
        store_function_arn(cached);

        assert_eq!(get_function_arn(), Some(cached.to_string()));
    }

    #[test]
    #[serial]
    fn constructs_arn_from_env_when_missing() {
        reset_state();
        *ACCOUNT_ID.lock() = Some("111122223333".to_string());
        env::set_var("AWS_REGION", "us-west-2");
        env::set_var("AWS_LAMBDA_FUNCTION_NAME", "my-function");

        let arn = get_function_arn();
        assert_eq!(
            arn.as_deref(),
            Some("arn:aws:lambda:us-west-2:111122223333:function:my-function")
        );

        // Ensure it is cached for subsequent calls even if env vars are removed
        env::remove_var("AWS_REGION");
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
        assert_eq!(
            get_function_arn(),
            Some("arn:aws:lambda:us-west-2:111122223333:function:my-function".to_string())
        );
    }

    #[test]
    #[serial]
    fn returns_none_when_any_component_missing() {
        reset_state();
        // Region present but account id missing
        env::set_var("AWS_REGION", "eu-central-1");
        env::set_var("AWS_LAMBDA_FUNCTION_NAME", "missing-account");

        assert_eq!(get_function_arn(), None);
        assert!(FUNCTION_ARN.lock().is_none());

        // Account present but function name missing
        *ACCOUNT_ID.lock() = Some("444455556666".to_string());
        env::remove_var("AWS_LAMBDA_FUNCTION_NAME");
        assert_eq!(get_function_arn(), None);
    }
}

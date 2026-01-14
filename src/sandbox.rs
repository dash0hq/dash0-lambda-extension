use aws_config::BehaviorVersion;
use aws_sdk_sts::Client as StsClient;
use hyper::{Body, Error, Request, Response};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::time::Instant;

/// Send a request through a {hyper::Client}
pub async fn send_request(request: Request<Body>) -> Result<Response<Body>, Error> {
    hyper::Client::new().request(request).await
}

/// Lambda Extensions API
///
/// Interact with the Lambda sandbox as a Lambda Extension
///
#[allow(dead_code)]
pub mod extension {
    use crate::DEFAULT_PROXY_PORT;
    use hyper::Body;
    use std::time::{Duration, Instant};

    /// Canonical Lambda Extensions API version
    ///
    /// Documentation: https://docs.aws.amazon.com/lambda/latest/dg/runtimes-extensions-api.html
    ///
    const EXTENSION_API_VERSION: &str = "2020-01-01";

    fn make_uri(path: &str) -> hyper::Uri {
        match hyper::Uri::builder()
            .scheme("http")
            .authority(crate::config::endpoints::sandbox_runtime_api())
            .path_and_query(format!("/{}/extension{}", EXTENSION_API_VERSION, path))
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

    /// Get next event from the Lambda Extensions API
    ///
    pub async fn get_next() {
        let uri = make_uri("/event/next");

        let mut request = match hyper::Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
        {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(
                    "[{}] Cannot create Lambda Extensions API request for get_next: {}",
                    crate::log_prefix_with("Extension"),
                    e
                );
                return;
            }
        };

        match crate::extension::register::extension_id().try_into() {
            Ok(header_value) => {
                request
                    .headers_mut()
                    .insert("Lambda-Extension-Identifier", header_value);
            }
            Err(e) => {
                tracing::error!(
                    "[{}] Invalid extension identifier for get_next: {}",
                    crate::log_prefix_with("Extension"),
                    e
                );
                return;
            }
        }

        let start = Instant::now();
        match super::send_request(request).await {
            Ok(response) => {
                let status = response.status();
                let body_bytes = match hyper::body::to_bytes(response.into_body()).await {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        tracing::error!(
                            "[{}] Failed to read extension event body: {}",
                            crate::log_prefix_with("Extension"),
                            err
                        );
                        return;
                    }
                };

                tracing::info!(
                    "[{}] Event status={} payload={} latency={} ms",
                    crate::log_prefix_with("Extension"),
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
                        let is_invocation_end = matches!(event_type, Some("SHUTDOWN"));
                        if is_invocation_end {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            crate::backend_send::flush_traces().await;
                        } else {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        crate::backend_send::flush_logs(is_invocation_end).await;
                    }

                    if matches!(event_type, Some("INVOKE"))
                        && crate::config::is_send_on_invocation_end()
                    {
                        // Block execution until platform.runtimeDone is received
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        crate::store::store_runtime_done_notifier(tx);

                        tracing::info!(
                            "[{}] Waiting for platform.runtimeDone",
                            crate::log_prefix_with("Extension")
                        );

                        // Wait for the signal with a timeout to prevent indefinite blocking
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(900), // 15 minute timeout (max Lambda duration)
                            rx,
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                tracing::info!(
                                    "[{}] Received platform.runtimeDone signal",
                                    crate::log_prefix_with("Extension")
                                );
                                crate::backend_send::flush_traces().await;
                                crate::backend_send::flush_logs(true).await;
                            }
                            Ok(Err(_)) => {
                                tracing::warn!(
                                    "[{}] platform.runtimeDone channel closed",
                                    crate::log_prefix_with("Extension")
                                );
                            }
                            Err(_) => {
                                tracing::error!(
                                    "[{}] Timeout waiting for platform.runtimeDone",
                                    crate::log_prefix_with("Extension")
                                );
                            }
                        }
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    "[{}] Error fetching next extension event: {}",
                    crate::log_prefix_with("Extension"),
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
            "[{}] Registering telemetry with payload={}",
            crate::log_prefix_with("Extension"),
            payload
        );

        let mut request = match hyper::Request::builder()
            .method("PUT")
            .uri(uri.clone())
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(hyper::Body::from(payload))
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

        match crate::extension::register::extension_id().try_into() {
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

        match super::send_request(request).await {
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

    fn make_telemetry_uri() -> hyper::Uri {
        match hyper::Uri::builder()
            .scheme("http")
            .authority(crate::config::endpoints::sandbox_runtime_api())
            .path_and_query("/2022-07-01/telemetry")
            .build()
        {
            Ok(uri) => uri,
            Err(e) => {
                tracing::error!(
                    "[{}] Error building Lambda Telemetry API endpoint URL: {}",
                    crate::log_prefix_with("Extension"),
                    e
                );
                panic!(
                    "[{}] Failed to build Telemetry API URI - severe misconfiguration: {}",
                    crate::log_prefix_with("Extension"),
                    e
                );
            }
        }
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
                    "[{}] Retrieved account ID={} from STS GetCallerIdentity in {} ms",
                    crate::log_prefix(),
                    account,
                    start.elapsed().as_millis()
                );
                Some(account)
            } else {
                tracing::error!(
                    "[{}] STS GetCallerIdentity did not return an account ID",
                    crate::log_prefix()
                );
                None
            }
        }
        Err(err) => {
            tracing::error!(
                "[{}] Failed calling STS GetCallerIdentity: {}",
                crate::log_prefix(),
                err
            );
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

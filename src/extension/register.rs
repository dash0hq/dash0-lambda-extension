use hyper::Body;

use crate::config::endpoints;

/// Canonical Lambda Extensions API version
const EXTENSION_API_VERSION: &str = "2020-01-01";

fn find_extension_name() -> String {
    crate::EXTENSION_NAME.to_owned()
}

fn make_uri(path: &str) -> hyper::Uri {
    match hyper::Uri::builder()
        .scheme("http")
        .authority(endpoints::sandbox_runtime_api())
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

/// Register the extension with the Lambda Extensions API
pub async fn register() {
    let uri = make_uri("/register");

    let body = Body::from(r#"{"events":["INVOKE","SHUTDOWN"]}"#);
    let mut request = match hyper::Request::builder()
        .method("POST")
        .uri(uri)
        .body(body)
    {
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

    let response = match crate::sandbox::send_request(request).await {
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
            tracing::error!("[{}] Lambda Extensions API response missing 'lambda-extension-identifier' header", crate::log_prefix_with("Extension"));
            panic!(
                "[{}] Extension registration failed - missing identifier header",
                crate::log_prefix_with("Extension")
            );
        }
    };

    crate::sandbox::extension::set_extension_identifier(extension_identifier);
}

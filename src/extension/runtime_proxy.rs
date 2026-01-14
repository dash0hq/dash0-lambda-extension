use std::sync::Arc;
use std::time::Duration;

use hyper::{Body, Error, Request, Response};

use crate::config::max_event_payload_size;
use crate::store::{store_current_invocation_id, store_event_payload};

/// Pass-through the request, but log the unhandled path and method
#[allow(dead_code)]
pub async fn notfound_passthru_proxy(req: Request<Body>) -> Result<Response<Body>, Error> {
    tracing::info!(
        "[{}] Route not found: path={} method={}",
        crate::log_prefix(),
        &req.uri().path(),
        &req.method()
    );
    crate::route::passthru_proxy(req).await
}

pub async fn proxy_invocation_next(req: Request<Body>) -> Result<Response<Body>, Error> {
    'getNext: loop {
        // track either initialization  -or-
        // how long it took to process the event and request next
        //
        crate::stats::get_next_event();

        let (aws_request_id, response) =
            match crate::sandbox::next(req.headers(), req.uri().path()).await {
                Err(e) => {
                    tracing::error!(
                        "[{}]  Error getting next invocation from Runtime API: {}",
                        crate::log_prefix(),
                        e
                    );
                    tracing::trace!("[{}] uri: {}", crate::log_prefix(), req.uri());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue 'getNext;
                }
                Ok(response) => response,
            };

        // start the counter on the new event
        crate::stats::event_start();

        store_current_invocation_id(aws_request_id.as_str());

        tracing::info!(
            "[{}] Got invocation next: {}",
            crate::log_prefix(),
            aws_request_id.as_str()
        );

        match validate_and_mangle_next_event(aws_request_id, response).await {
            Ok(response) => {
                return Ok(response);
            }
            Err(req) => {
                crate::sandbox::send_request(req).await.ok();
                continue 'getNext;
            }
        }
    }
}

async fn validate_and_mangle_next_event(
    _aws_request_id: Arc<String>,
    response: Response<Body>,
) -> Result<Response<Body>, Request<Body>> {
    let (parts, body) = response.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap_or_else(|e| {
        tracing::error!(
            "[{}] Failed to read event payload body: {}",
            crate::log_prefix(),
            e
        );
        hyper::body::Bytes::new()
    });

    let max_size = max_event_payload_size();
    let truncated_bytes = if body_bytes.len() > max_size {
        tracing::info!(
            "[{}] Truncating event payload from {} to {} bytes.",
            crate::log_prefix(),
            body_bytes.len(),
            max_size
        );
        &body_bytes[..max_size]
    } else {
        &body_bytes
    };
    store_event_payload(&_aws_request_id, &String::from_utf8_lossy(truncated_bytes));

    // Reconstruct the response with the same parts and body
    let response = Response::from_parts(parts, Body::from(body_bytes));

    Ok(response)
}

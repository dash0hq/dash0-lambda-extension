use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Method, Request, Response};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use matchit::Router as MatchitRouter;
use once_cell::sync::Lazy;

use crate::extension::runtime_proxy;
use crate::extension::telemetry_receiver::telemetry;
use crate::otlp::logs_receiver::logs;
use crate::otlp::metrics_receiver::metrics;
use crate::otlp::receiver::traces;
use crate::state::invocation_entry;

/// Body type used for all responses produced by this proxy.
pub type ResBody = Full<Bytes>;

/// Outgoing request body type used by HTTP clients in this crate.
pub type ReqBody = Full<Bytes>;

type HandlerFut =
    Pin<Box<dyn Future<Output = Result<Response<ResBody>, hyper::Error>> + Send + 'static>>;
type Handler = fn(Request<Incoming>) -> HandlerFut;

fn boxed<F>(fut: F) -> HandlerFut
where
    F: Future<Output = Result<Response<ResBody>, hyper::Error>> + Send + 'static,
{
    Box::pin(fut)
}

fn h_passthru(req: Request<Incoming>) -> HandlerFut {
    boxed(runtime_proxy::passthru_proxy(req))
}
fn h_invocation_next(req: Request<Incoming>) -> HandlerFut {
    boxed(runtime_proxy::proxy_invocation_next(req))
}
fn h_invocation_response(req: Request<Incoming>) -> HandlerFut {
    boxed(runtime_proxy::invocation_response_proxy(req))
}
fn h_traces(req: Request<Incoming>) -> HandlerFut {
    boxed(traces(req))
}
fn h_logs(req: Request<Incoming>) -> HandlerFut {
    boxed(logs(req))
}
fn h_metrics(req: Request<Incoming>) -> HandlerFut {
    boxed(metrics(req))
}
fn h_telemetry(req: Request<Incoming>) -> HandlerFut {
    boxed(telemetry(req))
}
fn h_notfound(req: Request<Incoming>) -> HandlerFut {
    boxed(runtime_proxy::notfound_passthru_proxy(req))
}

struct Routes {
    get: MatchitRouter<Handler>,
    post: MatchitRouter<Handler>,
}

static ROUTES: Lazy<Routes> = Lazy::new(|| {
    let mut get = MatchitRouter::new();
    let mut post = MatchitRouter::new();
    get.insert("/", h_passthru as Handler).unwrap();
    get.insert(
        "/:apiver/runtime/invocation/next",
        h_invocation_next as Handler,
    )
    .unwrap();
    post.insert(
        "/:apiver/runtime/invocation/:id/response",
        h_invocation_response as Handler,
    )
    .unwrap();
    post.insert(
        "/:apiver/runtime/invocation/:id/error",
        h_invocation_response as Handler,
    )
    .unwrap();
    post.insert("/:apiver/traces", h_traces as Handler).unwrap();
    post.insert("/:apiver/logs", h_logs as Handler).unwrap();
    post.insert("/:apiver/metrics", h_metrics as Handler)
        .unwrap();
    post.insert("/:apiver/telemetry", h_telemetry as Handler)
        .unwrap();
    Routes { get, post }
});

pub fn init() {
    Lazy::force(&ROUTES);
    Lazy::force(&HTTPS_CLIENT);
    invocation_entry::force_init();
}

pub async fn dispatch(req: Request<Incoming>) -> Result<Response<ResBody>, Infallible> {
    let table = match *req.method() {
        Method::GET => Some(&ROUTES.get),
        Method::POST => Some(&ROUTES.post),
        _ => None,
    };

    let handler = table
        .and_then(|t| t.at(req.uri().path()).ok().map(|m| *m.value))
        .unwrap_or(h_notfound as Handler);

    match handler(req).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            tracing::error!("[{}] Handler error: {}", crate::log_prefix(), e);
            Ok(Response::builder()
                .status(500)
                .body(Full::new(Bytes::from_static(b"500 - Internal Error")))
                .unwrap())
        }
    }
}

pub(crate) static HTTPS_CLIENT: Lazy<Client<hyper_rustls::HttpsConnector<HttpConnector>, ReqBody>> =
    Lazy::new(|| {
        let https = HttpsConnectorBuilder::new()
            .with_native_roots()
            .expect("failed to load native TLS root certificates")
            .https_only()
            .enable_http1()
            .build();
        Client::builder(TokioExecutor::new()).build(https)
    });

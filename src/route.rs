use httprouter::Router;
use hyper::Body;
use hyper_rustls::HttpsConnectorBuilder;
use once_cell::sync::Lazy;

use crate::extension::telemetry_receiver::telemetry;
use crate::otlp::logs_receiver::logs;
use crate::otlp::receiver::traces;
use crate::state::invocation_data::force_init_trace_store;

pub fn make_route<'a>() -> Router<'a> {
    let router = Router::default()
        .get("/", crate::extension::runtime_proxy::passthru_proxy)
        .get(
            "/:apiver/runtime/invocation/next",
            crate::extension::runtime_proxy::proxy_invocation_next,
        )
        .post(
            "/:apiver/runtime/invocation/:id/response",
            crate::extension::runtime_proxy::invocation_response_proxy,
        )
        .post(
            "/:apiver/runtime/invocation/:id/error",
            crate::extension::runtime_proxy::invocation_response_proxy,
        )
        .post("/:apiver/traces", traces)
        .post("/:apiver/logs", logs)
        .post("/:apiver/telemetry", telemetry)
        .not_found(crate::extension::runtime_proxy::notfound_passthru_proxy);
    Lazy::force(&HTTPS_CLIENT);
    force_init_trace_store();
    router
}

pub(crate) static HTTPS_CLIENT: Lazy<
    hyper::Client<hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>, Body>,
> = Lazy::new(|| {
    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_only()
        .enable_http1()
        .build();
    hyper::Client::builder().build::<_, Body>(https)
});

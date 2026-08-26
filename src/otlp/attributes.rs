// OpenTelemetry semantic convention attribute keys
pub const CLOUD_RESOURCE_ID: &str = "cloud.resource_id";
pub const CLOUD_ACCOUNT_ID: &str = "cloud.account.id";
pub const CLOUD_PLATFORM: &str = "cloud.platform";
pub const SERVICE_NAME: &str = "service.name";
pub const FAAS_INVOCATION_ID: &str = "faas.invocation_id";
pub const FAAS_TRIGGER: &str = "faas.trigger";
pub const FAAS_INIT_DURATION: &str = "faas.init_duration";

// Exception attributes
pub const EXCEPTION_TYPE: &str = "exception.type";
pub const EXCEPTION_MESSAGE: &str = "exception.message";
pub const EXCEPTION_ESCAPED: &str = "exception.escaped";
pub const EXCEPTION_STACKTRACE: &str = "exception.stacktrace";

// HTTP attributes
pub const HTTP_REQUEST_BODY: &str = "http.request.body";
pub const HTTP_RESPONSE_BODY: &str = "http.response.body";
pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
pub const HTTP_ROUTE: &str = "http.route";
pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
pub const URL_PATH: &str = "url.path";
pub const URL_SCHEME: &str = "url.scheme";
pub const URL_QUERY: &str = "url.query";
pub const SERVER_ADDRESS: &str = "server.address";
pub const SERVER_PORT: &str = "server.port";
pub const CLIENT_ADDRESS: &str = "client.address";
pub const NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
pub fn http_request_header(name: &str) -> String {
    format!("http.request.header.{}", name)
}
pub fn http_response_header(name: &str) -> String {
    format!("http.response.header.{}", name)
}

// Dash0-specific attributes
pub const DASH0_FAAS_RECORD_COUNT: &str = "dash0.faas.record_count";
pub const DASH0_FAAS_TRIGGER_ARN: &str = "dash0.faas.trigger_arn";
pub const DASH0_FAAS_EVENT_BRIDGE_SOURCE: &str = "dash0.faas.event_bridge_source";
pub const DASH0_FAAS_EVENT_BRIDGE_DETAIL_TYPE: &str = "dash0.faas.event_bridge_detail_type";
pub const DASH0_FAAS_PAYLOAD_TYPE: &str = "dash0.faas.payload_type";
pub const DASH0_FAAS_X_AMZN_TRACE_ID: &str = "dash0.faas.x_amzn_trace_id";

// Dash0 trigger chain attributes
pub const DASH0_TRIGGER_CHAIN_DEPTH: &str = "dash0.trigger.chain.depth";
pub const DASH0_TRIGGER_CHAIN_TRUNCATED: &str = "dash0.trigger.chain.truncated";

pub fn dash0_trigger_chain_type(index: usize) -> String {
    format!("dash0.trigger.chain.{}.type", index)
}

pub fn dash0_trigger_chain_arn(index: usize) -> String {
    format!("dash0.trigger.chain.{}.arn", index)
}

pub fn dash0_trigger_chain_name(index: usize) -> String {
    format!("dash0.trigger.chain.{}.name", index)
}

pub fn dash0_trigger_chain_timestamp(index: usize) -> String {
    format!("dash0.trigger.chain.{}.timestamp", index)
}

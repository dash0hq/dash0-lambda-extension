// OpenTelemetry semantic convention attribute keys
pub const CLOUD_RESOURCE_ID: &str = "cloud.resource_id";
pub const CLOUD_ACCOUNT_ID: &str = "cloud.account.id";
pub const CLOUD_PLATFORM: &str = "cloud.platform";
pub const CLOUD_RESOURCE_ID_SEMCONV: &str = "cloud.resource.id";
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

// Dash0-specific attributes
pub const DASH0_FAAS_RECORD_COUNT: &str = "dash0.faas.record_count";
pub const DASH0_FAAS_TRIGGER_ARN: &str = "dash0.faas.trigger_arn";
pub const DASH0_FAAS_EVENT_BRIDGE_SOURCE: &str = "dash0.faas.event_bridge_source";
pub const DASH0_FAAS_EVENT_BRIDGE_DETAIL_TYPE: &str = "dash0.faas.event_bridge_detail_type";
pub const DASH0_FAAS_PAYLOAD_TYPE: &str = "dash0.faas.payload_type";

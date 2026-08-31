//! HTTP semantic-convention attribute extraction for API Gateway-triggered
//! invocations (REST API v1 and HTTP API v2 proxy integration event shapes).
//!
//! Runs independently of the in-function runtime SDK: the extension already
//! buffers the raw invoke event and the raw return payload for every
//! invocation (see `extension::runtime_proxy`), so this works the same way
//! for every Lambda runtime, and also covers the synthetic-trace path used
//! when auto-instrumentation is disabled.

use std::collections::HashMap;

use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueEnum;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use serde_json::Value;

use crate::otlp::attributes::*;

pub enum ApiGatewayVersion {
    V1,
    V2,
}

fn string_kv(key: &str, value: String) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(AnyValueEnum::StringValue(value)),
        }),
    }
}

fn int_kv(key: &str, value: i64) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(AnyValueEnum::IntValue(value)),
        }),
    }
}

fn string_value(kv: &KeyValue) -> Option<String> {
    match &kv.value {
        Some(AnyValue {
            value: Some(AnyValueEnum::StringValue(s)),
        }) => Some(s.clone()),
        _ => None,
    }
}

/// `{proxy+}` -> `:proxy`, `{id}` -> `:id`, matching the normalized
/// `http.route` form used elsewhere (e.g. `@opentelemetry/instrumentation-http`).
fn normalize_route(route: &str) -> String {
    let mut result = String::with_capacity(route.len());
    let mut chars = route.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut name = String::new();
            for c2 in chars.by_ref() {
                if c2 == '}' {
                    break;
                }
                name.push(c2);
            }
            let name = name.trim_end_matches('+');
            result.push(':');
            result.push_str(name);
        } else {
            result.push(c);
        }
    }
    result
}

fn is_alb_event(json_val: &Value) -> bool {
    json_val
        .get("requestContext")
        .and_then(|rc| rc.get("elb"))
        .is_some_and(|elb| elb.is_object())
}

/// Detects whether `json_val` is an API Gateway REST API (v1) or HTTP API
/// (v2) proxy integration event. Returns `None` for ALB target-group events,
/// which also carry a `requestContext` but must not be misclassified.
pub fn detect_api_gateway_event(json_val: &Value) -> Option<ApiGatewayVersion> {
    if is_alb_event(json_val) {
        return None;
    }
    let rc = json_val.get("requestContext")?;
    if rc.get("http").is_some() && json_val.get("rawPath").is_some() {
        return Some(ApiGatewayVersion::V2);
    }
    if json_val
        .get("httpMethod")
        .and_then(|v| v.as_str())
        .is_some()
    {
        return Some(ApiGatewayVersion::V1);
    }
    None
}

/// Extracts request-side HTTP semconv attributes. No PII risk, populated
/// unconditionally (unlike headers/query string, which are opt-in).
pub fn extract_request_attributes(json_val: &Value, version: &ApiGatewayVersion) -> Vec<KeyValue> {
    let mut attrs = Vec::new();
    let rc = match json_val.get("requestContext") {
        Some(rc) => rc,
        None => return attrs,
    };

    match version {
        ApiGatewayVersion::V1 => {
            if let Some(method) = json_val.get("httpMethod").and_then(|v| v.as_str()) {
                attrs.push(string_kv(HTTP_REQUEST_METHOD, method.to_string()));
            }
            if let Some(path) = json_val.get("path").and_then(|v| v.as_str()) {
                attrs.push(string_kv(URL_PATH, path.to_string()));
            }
            attrs.push(string_kv(URL_SCHEME, "https".to_string()));
            if let Some(resource) = json_val.get("resource").and_then(|v| v.as_str()) {
                attrs.push(string_kv(HTTP_ROUTE, normalize_route(resource)));
            }
            if let Some(domain) = rc.get("domainName").and_then(|v| v.as_str()) {
                attrs.push(string_kv(SERVER_ADDRESS, domain.to_string()));
                attrs.push(int_kv(SERVER_PORT, 443));
            }
            if let Some(ip) = rc
                .get("identity")
                .and_then(|i| i.get("sourceIp"))
                .and_then(|v| v.as_str())
            {
                attrs.push(string_kv(CLIENT_ADDRESS, ip.to_string()));
            }
            if let Some(protocol) = rc.get("protocol").and_then(|v| v.as_str()) {
                if let Some(protocol_version) = protocol.split('/').nth(1) {
                    attrs.push(string_kv(
                        NETWORK_PROTOCOL_VERSION,
                        protocol_version.to_string(),
                    ));
                }
            }
        }
        ApiGatewayVersion::V2 => {
            let http = rc.get("http");
            if let Some(method) = http.and_then(|h| h.get("method")).and_then(|v| v.as_str()) {
                attrs.push(string_kv(HTTP_REQUEST_METHOD, method.to_string()));
            }
            if let Some(path) = json_val.get("rawPath").and_then(|v| v.as_str()) {
                attrs.push(string_kv(URL_PATH, path.to_string()));
            }
            attrs.push(string_kv(URL_SCHEME, "https".to_string()));
            if let Some(route_key) = rc.get("routeKey").and_then(|v| v.as_str()) {
                let route = route_key
                    .split_once(' ')
                    .map(|(_, r)| r)
                    .unwrap_or(route_key);
                attrs.push(string_kv(HTTP_ROUTE, normalize_route(route)));
            }
            if let Some(domain) = rc.get("domainName").and_then(|v| v.as_str()) {
                attrs.push(string_kv(SERVER_ADDRESS, domain.to_string()));
                attrs.push(int_kv(SERVER_PORT, 443));
            }
            if let Some(ip) = http
                .and_then(|h| h.get("sourceIp"))
                .and_then(|v| v.as_str())
            {
                attrs.push(string_kv(CLIENT_ADDRESS, ip.to_string()));
            }
            if let Some(protocol) = http
                .and_then(|h| h.get("protocol"))
                .and_then(|v| v.as_str())
            {
                if let Some(protocol_version) = protocol.split('/').nth(1) {
                    attrs.push(string_kv(
                        NETWORK_PROTOCOL_VERSION,
                        protocol_version.to_string(),
                    ));
                }
            }
        }
    }

    attrs
}

/// Builds `"<METHOD> <route>"`, used only when span-naming is opted in via
/// `DASH0_ENABLE_API_GATEWAY_SPAN_NAME`.
pub fn extract_span_name(json_val: &Value, version: &ApiGatewayVersion) -> Option<String> {
    let attrs = extract_request_attributes(json_val, version);
    let method = attrs
        .iter()
        .find(|kv| kv.key == HTTP_REQUEST_METHOD)
        .and_then(string_value)?;
    let route = attrs
        .iter()
        .find(|kv| kv.key == HTTP_ROUTE)
        .and_then(string_value)?;
    Some(format!("{} {}", method, route))
}

/// Extracts `http.response.status_code` from a Lambda proxy-integration
/// return payload (`{ statusCode, headers, body }`), if present.
pub fn extract_response_status_code_attribute(return_value_json: &Value) -> Option<KeyValue> {
    return_value_json
        .get("statusCode")
        .and_then(|v| v.as_i64())
        .map(|code| int_kv(HTTP_RESPONSE_STATUS_CODE, code))
}

fn parse_allow_list(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Captures only allow-listed headers (case-insensitive), gated by an
/// explicit allow-list per HTTP semconv: instrumentations must not capture
/// headers by default.
pub fn extract_header_attributes(
    headers: Option<&Value>,
    allow_list_csv: &str,
    prefix_fn: fn(&str) -> String,
) -> Vec<KeyValue> {
    let allow_list = parse_allow_list(allow_list_csv);
    if allow_list.is_empty() {
        return Vec::new();
    }
    let headers = match headers.and_then(|h| h.as_object()) {
        Some(h) => h,
        None => return Vec::new(),
    };
    let mut lower_cased: HashMap<String, &Value> = HashMap::new();
    for (name, value) in headers {
        lower_cased.insert(name.to_lowercase(), value);
    }

    allow_list
        .iter()
        .filter_map(|name| {
            lower_cased
                .get(name)
                .and_then(|v| v.as_str())
                .map(|s| string_kv(&prefix_fn(name), s.to_string()))
        })
        .collect()
}

/// Extracts `url.query`, gated by `DASH0_CAPTURE_API_GATEWAY_QUERY_STRING`
/// since query strings can carry signed-URL tokens or other secrets.
pub fn extract_query_string_attribute(
    json_val: &Value,
    version: &ApiGatewayVersion,
) -> Option<KeyValue> {
    match version {
        ApiGatewayVersion::V1 => {
            let params = json_val
                .get("multiValueQueryStringParameters")?
                .as_object()?;
            let mut parts = Vec::new();
            for (key, values) in params {
                if let Some(values) = values.as_array() {
                    for value in values {
                        if let Some(value) = value.as_str() {
                            parts.push(format!("{}={}", key, value));
                        }
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(string_kv(URL_QUERY, parts.join("&")))
            }
        }
        ApiGatewayVersion::V2 => json_val
            .get("rawQueryString")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| string_kv(URL_QUERY, s.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1_event() -> Value {
        serde_json::json!({
            "httpMethod": "GET",
            "path": "/pets/123",
            "resource": "/pets/{id}",
            "headers": {"Content-Type": "application/json", "Authorization": "secret"},
            "multiValueQueryStringParameters": {"color": ["red", "blue"]},
            "requestContext": {
                "domainName": "abc123.execute-api.us-east-1.amazonaws.com",
                "identity": {"sourceIp": "1.2.3.4"},
                "protocol": "HTTP/1.1"
            }
        })
    }

    fn v2_event() -> Value {
        serde_json::json!({
            "rawPath": "/pets/123",
            "rawQueryString": "color=red",
            "headers": {"content-type": "application/json"},
            "requestContext": {
                "domainName": "abc123.execute-api.us-east-1.amazonaws.com",
                "http": {
                    "method": "GET",
                    "path": "/pets/123",
                    "protocol": "HTTP/1.1",
                    "sourceIp": "1.2.3.4"
                },
                "routeKey": "GET /pets/{id}"
            }
        })
    }

    fn get_str<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
        attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
            if let Some(AnyValue {
                value: Some(AnyValueEnum::StringValue(s)),
            }) = &kv.value
            {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    fn get_int(attrs: &[KeyValue], key: &str) -> Option<i64> {
        attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
            if let Some(AnyValue {
                value: Some(AnyValueEnum::IntValue(i)),
            }) = &kv.value
            {
                Some(*i)
            } else {
                None
            }
        })
    }

    #[test]
    fn detects_v1_event() {
        assert!(matches!(
            detect_api_gateway_event(&v1_event()),
            Some(ApiGatewayVersion::V1)
        ));
    }

    #[test]
    fn detects_v2_event() {
        assert!(matches!(
            detect_api_gateway_event(&v2_event()),
            Some(ApiGatewayVersion::V2)
        ));
    }

    #[test]
    fn does_not_detect_non_api_gateway_events() {
        assert!(detect_api_gateway_event(&serde_json::json!({"Records": []})).is_none());
    }

    #[test]
    fn does_not_misclassify_alb_events() {
        let alb_event = serde_json::json!({
            "httpMethod": "GET",
            "path": "/lambda",
            "requestContext": {"elb": {"targetGroupArn": "arn:aws:elasticloadbalancing:..."}}
        });
        assert!(detect_api_gateway_event(&alb_event).is_none());
    }

    #[test]
    fn extracts_v1_request_attributes_and_normalizes_route() {
        let attrs = extract_request_attributes(&v1_event(), &ApiGatewayVersion::V1);
        assert_eq!(get_str(&attrs, HTTP_REQUEST_METHOD), Some("GET"));
        assert_eq!(get_str(&attrs, URL_PATH), Some("/pets/123"));
        assert_eq!(get_str(&attrs, URL_SCHEME), Some("https"));
        assert_eq!(get_str(&attrs, HTTP_ROUTE), Some("/pets/:id"));
        assert_eq!(
            get_str(&attrs, SERVER_ADDRESS),
            Some("abc123.execute-api.us-east-1.amazonaws.com")
        );
        assert_eq!(get_int(&attrs, SERVER_PORT), Some(443));
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("1.2.3.4"));
        assert_eq!(get_str(&attrs, NETWORK_PROTOCOL_VERSION), Some("1.1"));
    }

    #[test]
    fn extracts_v2_request_attributes_and_normalizes_route() {
        let attrs = extract_request_attributes(&v2_event(), &ApiGatewayVersion::V2);
        assert_eq!(get_str(&attrs, HTTP_REQUEST_METHOD), Some("GET"));
        assert_eq!(get_str(&attrs, URL_PATH), Some("/pets/123"));
        assert_eq!(get_str(&attrs, HTTP_ROUTE), Some("/pets/:id"));
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("1.2.3.4"));
        assert_eq!(get_str(&attrs, NETWORK_PROTOCOL_VERSION), Some("1.1"));
    }

    #[test]
    fn handles_default_v2_route_key_without_leading_method() {
        let mut event = v2_event();
        event["requestContext"]["routeKey"] = serde_json::json!("$default");
        let attrs = extract_request_attributes(&event, &ApiGatewayVersion::V2);
        assert_eq!(get_str(&attrs, HTTP_ROUTE), Some("$default"));
    }

    #[test]
    fn builds_span_name_for_v1_and_v2() {
        assert_eq!(
            extract_span_name(&v1_event(), &ApiGatewayVersion::V1),
            Some("GET /pets/:id".to_string())
        );
        assert_eq!(
            extract_span_name(&v2_event(), &ApiGatewayVersion::V2),
            Some("GET /pets/:id".to_string())
        );
    }

    #[test]
    fn span_name_none_without_route() {
        let mut event = v1_event();
        event.as_object_mut().unwrap().remove("resource");
        assert!(extract_span_name(&event, &ApiGatewayVersion::V1).is_none());
    }

    #[test]
    fn extracts_response_status_code() {
        let response = serde_json::json!({"statusCode": 404, "body": "not found"});
        let kv = extract_response_status_code_attribute(&response).unwrap();
        assert_eq!(kv.key, HTTP_RESPONSE_STATUS_CODE);
        assert_eq!(
            get_int(std::slice::from_ref(&kv), HTTP_RESPONSE_STATUS_CODE),
            Some(404)
        );
    }

    #[test]
    fn no_response_status_code_when_absent() {
        let response = serde_json::json!({"body": "not a proxy result"});
        assert!(extract_response_status_code_attribute(&response).is_none());
    }

    #[test]
    fn header_capture_empty_allow_list_returns_nothing() {
        let attrs = extract_header_attributes(v1_event().get("headers"), "", http_request_header);
        assert!(attrs.is_empty());
    }

    #[test]
    fn header_capture_matches_case_insensitively_and_only_allow_listed() {
        let attrs = extract_header_attributes(
            v1_event().get("headers"),
            "content-type",
            http_request_header,
        );
        assert_eq!(attrs.len(), 1);
        assert_eq!(
            get_str(&attrs, &http_request_header("content-type")),
            Some("application/json")
        );
    }

    #[test]
    fn query_string_flattens_v1_multi_value_params() {
        let kv = extract_query_string_attribute(&v1_event(), &ApiGatewayVersion::V1).unwrap();
        assert_eq!(
            get_str(std::slice::from_ref(&kv), URL_QUERY),
            Some("color=red&color=blue")
        );
    }

    #[test]
    fn query_string_uses_v2_raw_query_string_as_is() {
        let kv = extract_query_string_attribute(&v2_event(), &ApiGatewayVersion::V2).unwrap();
        assert_eq!(
            get_str(std::slice::from_ref(&kv), URL_QUERY),
            Some("color=red")
        );
    }

    #[test]
    fn query_string_none_when_absent() {
        let mut event = v2_event();
        event["rawQueryString"] = serde_json::json!("");
        assert!(extract_query_string_attribute(&event, &ApiGatewayVersion::V2).is_none());
    }
}

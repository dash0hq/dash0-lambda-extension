//! HTTP semantic-convention attribute extraction for HTTP-triggered
//! invocations: API Gateway REST API (v1) and HTTP API (v2) proxy integration
//! event shapes, and Application Load Balancer target-group events.
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

/// The kind of HTTP-fronted trigger an invoke event came from. Each shape
/// carries the same information under different field names, so extraction
/// branches on this rather than probing fields ad hoc.
pub enum HttpEventKind {
    /// API Gateway REST API, proxy integration.
    ApiGatewayV1,
    /// API Gateway HTTP API, payload format version 2.0.
    ApiGatewayV2,
    /// Application Load Balancer target-group event.
    Alb,
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

fn is_alb_event(json_val: &Value) -> bool {
    json_val
        .get("requestContext")
        .and_then(|rc| rc.get("elb"))
        .is_some_and(|elb| elb.is_object())
}

/// Detects which HTTP-fronted trigger shape `json_val` is, if any.
///
/// ALB must be checked first: its events also carry a `requestContext` and a
/// top-level `httpMethod`, so they would otherwise fall into the v1 branch and
/// be reported as API Gateway. The `requestContext.elb` object is the
/// discriminator — it is present on every ALB event, including health checks.
pub fn detect_http_event(json_val: &Value) -> Option<HttpEventKind> {
    if is_alb_event(json_val) {
        return Some(HttpEventKind::Alb);
    }
    let rc = json_val.get("requestContext")?;
    if rc.get("http").is_some() && json_val.get("rawPath").is_some() {
        return Some(HttpEventKind::ApiGatewayV2);
    }
    if json_val
        .get("httpMethod")
        .and_then(|v| v.as_str())
        .is_some()
    {
        return Some(HttpEventKind::ApiGatewayV1);
    }
    None
}

/// Resolves the headers object of an event or return payload.
///
/// ALB target groups with `lambda.multi_value_headers.enabled` send
/// `multiValueHeaders` (name -> array of values) *instead of* `headers`, not
/// alongside it, in both directions. Reading only `headers` therefore yields
/// nothing at all on such target groups.
pub fn resolve_headers(json_val: &Value) -> Option<&Value> {
    json_val
        .get("headers")
        .filter(|v| v.is_object())
        .or_else(|| json_val.get("multiValueHeaders").filter(|v| v.is_object()))
}

/// Looks up a single header value, tolerating both the string-valued
/// (`headers`) and array-valued (`multiValueHeaders`) shapes. ALB lower-cases
/// request header names, but the match is case-insensitive regardless.
fn header_str<'a>(headers: Option<&'a Value>, name: &str) -> Option<&'a str> {
    let obj = headers?.as_object()?;
    let value = obj.get(name).or_else(|| {
        obj.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })?;
    match value {
        Value::String(s) => Some(s.as_str()),
        Value::Array(values) => values.first().and_then(|v| v.as_str()),
        _ => None,
    }
}

/// Splits a `Host` header into host and optional port. IPv6 literals are
/// bracketed (`[::1]:8080`), so only a colon after the closing bracket counts.
fn split_host_port(host: &str) -> (&str, Option<i64>) {
    let search_from = host.rfind(']').map(|i| i + 1).unwrap_or(0);
    match host[search_from..].rfind(':') {
        Some(offset) => {
            let index = search_from + offset;
            match host[index + 1..].parse::<i64>() {
                Ok(port) => (&host[..index], Some(port)),
                Err(_) => (host, None),
            }
        }
        None => (host, None),
    }
}

/// Extracts request-side HTTP semconv attributes. No PII risk, populated
/// unconditionally (unlike headers/query string, which are opt-in).
pub fn extract_request_attributes(json_val: &Value, kind: &HttpEventKind) -> Vec<KeyValue> {
    let mut attrs = Vec::new();
    let rc = match json_val.get("requestContext") {
        Some(rc) => rc,
        None => return attrs,
    };

    match kind {
        HttpEventKind::Alb => {
            // ALB carries none of API Gateway's routing metadata: there is no
            // `resource`/`routeKey` (so no `http.route`, and hence no
            // low-cardinality span name) and no `protocol` (so no
            // `network.protocol.version`). Everything below the method and
            // path comes from the forwarding headers ALB adds to every
            // request. Health-check events carry only `user-agent`, so each
            // attribute is independently optional.
            let headers = resolve_headers(json_val);

            if let Some(method) = json_val.get("httpMethod").and_then(|v| v.as_str()) {
                attrs.push(string_kv(HTTP_REQUEST_METHOD, method.to_string()));
            }
            if let Some(path) = json_val.get("path").and_then(|v| v.as_str()) {
                attrs.push(string_kv(URL_PATH, path.to_string()));
            }
            // Not hardcoded to "https" as in the API Gateway branches: ALB
            // listeners accept plain HTTP too, and the original client scheme
            // is what `x-forwarded-proto` reports.
            if let Some(scheme) = header_str(headers, "x-forwarded-proto") {
                attrs.push(string_kv(URL_SCHEME, scheme.to_string()));
            }
            if let Some(host) = header_str(headers, "host") {
                let (address, port_in_host) = split_host_port(host);
                if !address.is_empty() {
                    attrs.push(string_kv(SERVER_ADDRESS, address.to_string()));
                }
                // A port in the Host header is what the client actually
                // addressed, so it wins over the listener port ALB reports.
                let port = port_in_host.or_else(|| {
                    header_str(headers, "x-forwarded-port").and_then(|p| p.parse::<i64>().ok())
                });
                if let Some(port) = port {
                    attrs.push(int_kv(SERVER_PORT, port));
                }
            }
            // `x-forwarded-for` accumulates proxy hops left to right; the
            // original client is the first entry.
            if let Some(ip) = header_str(headers, "x-forwarded-for")
                .and_then(|v| v.split(',').next())
                .map(str::trim)
                .filter(|ip| !ip.is_empty())
            {
                attrs.push(string_kv(CLIENT_ADDRESS, ip.to_string()));
            }
        }
        HttpEventKind::ApiGatewayV1 => {
            if let Some(method) = json_val.get("httpMethod").and_then(|v| v.as_str()) {
                attrs.push(string_kv(HTTP_REQUEST_METHOD, method.to_string()));
            }
            if let Some(path) = json_val.get("path").and_then(|v| v.as_str()) {
                attrs.push(string_kv(URL_PATH, path.to_string()));
            }
            attrs.push(string_kv(URL_SCHEME, "https".to_string()));
            if let Some(resource) = json_val.get("resource").and_then(|v| v.as_str()) {
                attrs.push(string_kv(HTTP_ROUTE, resource.to_string()));
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
        HttpEventKind::ApiGatewayV2 => {
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
                attrs.push(string_kv(HTTP_ROUTE, route.to_string()));
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
///
/// ALB reports no route template, and `url.path` is high-cardinality, so per
/// HTTP semconv the name falls back to the bare method (e.g. `GET`) there.
pub fn extract_span_name(json_val: &Value, kind: &HttpEventKind) -> Option<String> {
    let attrs = extract_request_attributes(json_val, kind);
    let method = attrs
        .iter()
        .find(|kv| kv.key == HTTP_REQUEST_METHOD)
        .and_then(string_value)?;
    let route = attrs
        .iter()
        .find(|kv| kv.key == HTTP_ROUTE)
        .and_then(string_value);
    match route {
        Some(route) => Some(format!("{} {}", method, route)),
        None if matches!(kind, HttpEventKind::Alb) => Some(method),
        None => None,
    }
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
            let value = match lower_cased.get(name)? {
                Value::String(s) => s.clone(),
                // `multiValueHeaders` shape. Repeated field lines are folded
                // into one comma-separated value (RFC 9110 §5.3) so the
                // attribute stays a string, as it already is for the
                // single-value shape.
                Value::Array(values) => {
                    let parts: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();
                    if parts.is_empty() {
                        return None;
                    }
                    parts.join(", ")
                }
                _ => return None,
            };
            Some(string_kv(&prefix_fn(name), value))
        })
        .collect()
}

/// Extracts `url.query`, gated by `DASH0_CAPTURE_API_GATEWAY_QUERY_STRING`
/// since query strings can carry signed-URL tokens or other secrets.
pub fn extract_query_string_attribute(json_val: &Value, kind: &HttpEventKind) -> Option<KeyValue> {
    match kind {
        HttpEventKind::ApiGatewayV1 => {
            flatten_query_parameters(json_val.get("multiValueQueryStringParameters")?)
        }
        HttpEventKind::ApiGatewayV2 => json_val
            .get("rawQueryString")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| string_kv(URL_QUERY, s.to_string())),
        // Like headers, ALB sends `multiValueQueryStringParameters` instead of
        // `queryStringParameters` when multi-value headers are enabled.
        // Neither is URL-decoded by ALB, so the values reassemble into the
        // original query string as-is.
        HttpEventKind::Alb => {
            let params = json_val
                .get("queryStringParameters")
                .filter(|v| v.is_object())
                .or_else(|| {
                    json_val
                        .get("multiValueQueryStringParameters")
                        .filter(|v| v.is_object())
                })?;
            flatten_query_parameters(params)
        }
    }
}

/// Rebuilds a `key=value&key=value` query string from a parameter map whose
/// values are either strings or arrays of strings.
fn flatten_query_parameters(params: &Value) -> Option<KeyValue> {
    let params = params.as_object()?;
    let mut parts = Vec::new();
    for (key, value) in params {
        match value {
            Value::String(value) => parts.push(format!("{}={}", key, value)),
            Value::Array(values) => {
                for value in values {
                    if let Some(value) = value.as_str() {
                        parts.push(format!("{}={}", key, value));
                    }
                }
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(string_kv(URL_QUERY, parts.join("&")))
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

    /// Default ALB target group (multi-value headers disabled), taken from the
    /// example event in the Application Load Balancer user guide.
    fn alb_event() -> Value {
        serde_json::json!({
            "requestContext": {
                "elb": {
                    "targetGroupArn": "arn:aws:elasticloadbalancing:us-east-1:123456789012:targetgroup/lambda-279XGJDqGZ5rsrHC2Fjr/49e9d65c45c6791a"
                }
            },
            "httpMethod": "GET",
            "path": "/lambda",
            "queryStringParameters": {"query": "1234ABCD"},
            "headers": {
                "content-type": "application/json",
                "host": "lambda-alb-123578498.us-east-1.elb.amazonaws.com",
                "x-amzn-trace-id": "Root=1-5c536348-3d683b8b04734faae651f476",
                "x-forwarded-for": "72.12.164.125",
                "x-forwarded-port": "80",
                "x-forwarded-proto": "http"
            },
            "body": "",
            "isBase64Encoded": false
        })
    }

    /// Same request against a target group with
    /// `lambda.multi_value_headers.enabled`: ALB then sends
    /// `multiValueHeaders` / `multiValueQueryStringParameters` *instead of*
    /// the single-value fields, which are absent entirely.
    fn alb_event_multi_value() -> Value {
        serde_json::json!({
            "requestContext": {
                "elb": {"targetGroupArn": "arn:aws:elasticloadbalancing:us-east-1:123456789012:targetgroup/tg/49e9d65c45c6791a"}
            },
            "httpMethod": "POST",
            "path": "/lambda",
            "multiValueQueryStringParameters": {"myKey": ["val1", "val2"]},
            "multiValueHeaders": {
                "content-type": ["application/json"],
                "cookie": ["name1=value1", "name2=value2"],
                "host": ["lambda-alb-123578498.us-east-1.elb.amazonaws.com"],
                "x-forwarded-for": ["72.12.164.125"],
                "x-forwarded-port": ["443"],
                "x-forwarded-proto": ["https"]
            },
            "body": "",
            "isBase64Encoded": false
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
            detect_http_event(&v1_event()),
            Some(HttpEventKind::ApiGatewayV1)
        ));
    }

    #[test]
    fn detects_v2_event() {
        assert!(matches!(
            detect_http_event(&v2_event()),
            Some(HttpEventKind::ApiGatewayV2)
        ));
    }

    #[test]
    fn does_not_detect_non_api_gateway_events() {
        assert!(detect_http_event(&serde_json::json!({"Records": []})).is_none());
    }

    /// ALB events carry a top-level `httpMethod` just like API Gateway v1, so
    /// this guards the ordering inside `detect_http_event`.
    #[test]
    fn classifies_alb_events_as_alb_not_api_gateway_v1() {
        let alb_event = serde_json::json!({
            "httpMethod": "GET",
            "path": "/lambda",
            "requestContext": {"elb": {"targetGroupArn": "arn:aws:elasticloadbalancing:..."}}
        });
        assert!(matches!(
            detect_http_event(&alb_event),
            Some(HttpEventKind::Alb)
        ));
    }

    #[test]
    fn extracts_v1_request_attributes() {
        let attrs = extract_request_attributes(&v1_event(), &HttpEventKind::ApiGatewayV1);
        assert_eq!(get_str(&attrs, HTTP_REQUEST_METHOD), Some("GET"));
        assert_eq!(get_str(&attrs, URL_PATH), Some("/pets/123"));
        assert_eq!(get_str(&attrs, URL_SCHEME), Some("https"));
        assert_eq!(get_str(&attrs, HTTP_ROUTE), Some("/pets/{id}"));
        assert_eq!(
            get_str(&attrs, SERVER_ADDRESS),
            Some("abc123.execute-api.us-east-1.amazonaws.com")
        );
        assert_eq!(get_int(&attrs, SERVER_PORT), Some(443));
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("1.2.3.4"));
        assert_eq!(get_str(&attrs, NETWORK_PROTOCOL_VERSION), Some("1.1"));
    }

    #[test]
    fn extracts_v2_request_attributes() {
        let attrs = extract_request_attributes(&v2_event(), &HttpEventKind::ApiGatewayV2);
        assert_eq!(get_str(&attrs, HTTP_REQUEST_METHOD), Some("GET"));
        assert_eq!(get_str(&attrs, URL_PATH), Some("/pets/123"));
        assert_eq!(get_str(&attrs, HTTP_ROUTE), Some("/pets/{id}"));
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("1.2.3.4"));
        assert_eq!(get_str(&attrs, NETWORK_PROTOCOL_VERSION), Some("1.1"));
    }

    #[test]
    fn handles_default_v2_route_key_without_leading_method() {
        let mut event = v2_event();
        event["requestContext"]["routeKey"] = serde_json::json!("$default");
        let attrs = extract_request_attributes(&event, &HttpEventKind::ApiGatewayV2);
        assert_eq!(get_str(&attrs, HTTP_ROUTE), Some("$default"));
    }

    #[test]
    fn builds_span_name_for_v1_and_v2() {
        assert_eq!(
            extract_span_name(&v1_event(), &HttpEventKind::ApiGatewayV1),
            Some("GET /pets/{id}".to_string())
        );
        assert_eq!(
            extract_span_name(&v2_event(), &HttpEventKind::ApiGatewayV2),
            Some("GET /pets/{id}".to_string())
        );
    }

    #[test]
    fn span_name_none_without_route() {
        let mut event = v1_event();
        event.as_object_mut().unwrap().remove("resource");
        assert!(extract_span_name(&event, &HttpEventKind::ApiGatewayV1).is_none());
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
        let kv = extract_query_string_attribute(&v1_event(), &HttpEventKind::ApiGatewayV1).unwrap();
        assert_eq!(
            get_str(std::slice::from_ref(&kv), URL_QUERY),
            Some("color=red&color=blue")
        );
    }

    #[test]
    fn query_string_uses_v2_raw_query_string_as_is() {
        let kv = extract_query_string_attribute(&v2_event(), &HttpEventKind::ApiGatewayV2).unwrap();
        assert_eq!(
            get_str(std::slice::from_ref(&kv), URL_QUERY),
            Some("color=red")
        );
    }

    #[test]
    fn query_string_none_when_absent() {
        let mut event = v2_event();
        event["rawQueryString"] = serde_json::json!("");
        assert!(extract_query_string_attribute(&event, &HttpEventKind::ApiGatewayV2).is_none());
    }

    // ── ALB ───────────────────────────────────────────────────────────

    #[test]
    fn extracts_alb_request_attributes() {
        let attrs = extract_request_attributes(&alb_event(), &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, HTTP_REQUEST_METHOD), Some("GET"));
        assert_eq!(get_str(&attrs, URL_PATH), Some("/lambda"));
        assert_eq!(
            get_str(&attrs, SERVER_ADDRESS),
            Some("lambda-alb-123578498.us-east-1.elb.amazonaws.com")
        );
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("72.12.164.125"));
    }

    /// ALB listeners serve plain HTTP as well, so the scheme must follow
    /// `x-forwarded-proto` rather than being hardcoded to https the way the
    /// API Gateway branches do.
    #[test]
    fn alb_scheme_and_port_follow_forwarded_headers() {
        let attrs = extract_request_attributes(&alb_event(), &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, URL_SCHEME), Some("http"));
        assert_eq!(get_int(&attrs, SERVER_PORT), Some(80));

        let attrs = extract_request_attributes(&alb_event_multi_value(), &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, URL_SCHEME), Some("https"));
        assert_eq!(get_int(&attrs, SERVER_PORT), Some(443));
    }

    #[test]
    fn alb_multi_value_headers_are_read_when_single_value_headers_are_absent() {
        let event = alb_event_multi_value();
        let attrs = extract_request_attributes(&event, &HttpEventKind::Alb);
        assert_eq!(
            get_str(&attrs, SERVER_ADDRESS),
            Some("lambda-alb-123578498.us-east-1.elb.amazonaws.com")
        );
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("72.12.164.125"));
    }

    /// A port in the Host header is what the client actually addressed and
    /// wins over the listener port ALB reports in `x-forwarded-port`.
    #[test]
    fn alb_host_header_port_takes_precedence() {
        let mut event = alb_event();
        event["headers"]["host"] = serde_json::json!("example.com:8443");
        let attrs = extract_request_attributes(&event, &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, SERVER_ADDRESS), Some("example.com"));
        assert_eq!(get_int(&attrs, SERVER_PORT), Some(8443));
    }

    #[test]
    fn alb_ipv6_host_header_is_not_split_on_its_own_colons() {
        let mut event = alb_event();
        event["headers"]["host"] = serde_json::json!("[2001:db8::1]:8443");
        let attrs = extract_request_attributes(&event, &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, SERVER_ADDRESS), Some("[2001:db8::1]"));
        assert_eq!(get_int(&attrs, SERVER_PORT), Some(8443));

        let mut event = alb_event();
        event["headers"]["host"] = serde_json::json!("[2001:db8::1]");
        let attrs = extract_request_attributes(&event, &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, SERVER_ADDRESS), Some("[2001:db8::1]"));
    }

    #[test]
    fn alb_client_address_takes_first_forwarded_for_hop() {
        let mut event = alb_event();
        event["headers"]["x-forwarded-for"] =
            serde_json::json!("72.12.164.125, 10.0.0.1, 10.0.0.2");
        let attrs = extract_request_attributes(&event, &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), Some("72.12.164.125"));
    }

    /// ALB reports no route template, so `http.route` must be absent rather
    /// than guessed from the high-cardinality path.
    #[test]
    fn alb_has_no_route_or_protocol_version() {
        let attrs = extract_request_attributes(&alb_event(), &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, HTTP_ROUTE), None);
        assert_eq!(get_str(&attrs, NETWORK_PROTOCOL_VERSION), None);
    }

    /// Health checks (`ELB-HealthChecker/2.0`) carry no host and no forwarding
    /// headers at all, so extraction must degrade to method and path.
    #[test]
    fn alb_health_check_event_yields_only_method_and_path() {
        let event = serde_json::json!({
            "requestContext": {"elb": {"targetGroupArn": "arn:aws:elasticloadbalancing:us-east-1:123456789012:targetgroup/tg/abc"}},
            "httpMethod": "GET",
            "path": "/",
            "queryStringParameters": {},
            "headers": {"user-agent": "ELB-HealthChecker/2.0"},
            "body": "",
            "isBase64Encoded": false
        });
        let attrs = extract_request_attributes(&event, &HttpEventKind::Alb);
        assert_eq!(get_str(&attrs, HTTP_REQUEST_METHOD), Some("GET"));
        assert_eq!(get_str(&attrs, URL_PATH), Some("/"));
        assert_eq!(get_str(&attrs, SERVER_ADDRESS), None);
        assert_eq!(get_str(&attrs, CLIENT_ADDRESS), None);
        assert_eq!(get_str(&attrs, URL_SCHEME), None);
    }

    /// No route means no `<METHOD> <route>`; semconv falls back to the bare
    /// method rather than producing nothing.
    #[test]
    fn alb_span_name_falls_back_to_method() {
        assert_eq!(
            extract_span_name(&alb_event(), &HttpEventKind::Alb),
            Some("GET".to_string())
        );
    }

    #[test]
    fn alb_query_string_handles_both_shapes() {
        let kv = extract_query_string_attribute(&alb_event(), &HttpEventKind::Alb).unwrap();
        assert_eq!(
            get_str(std::slice::from_ref(&kv), URL_QUERY),
            Some("query=1234ABCD")
        );

        let kv =
            extract_query_string_attribute(&alb_event_multi_value(), &HttpEventKind::Alb).unwrap();
        assert_eq!(
            get_str(std::slice::from_ref(&kv), URL_QUERY),
            Some("myKey=val1&myKey=val2")
        );
    }

    #[test]
    fn alb_query_string_none_when_empty() {
        let mut event = alb_event();
        event["queryStringParameters"] = serde_json::json!({});
        assert!(extract_query_string_attribute(&event, &HttpEventKind::Alb).is_none());
    }

    #[test]
    fn alb_header_capture_folds_repeated_multi_value_entries() {
        let event = alb_event_multi_value();
        let attrs =
            extract_header_attributes(resolve_headers(&event), "cookie", http_request_header);
        assert_eq!(
            get_str(&attrs, &http_request_header("cookie")),
            Some("name1=value1, name2=value2")
        );
    }

    #[test]
    fn alb_header_capture_reads_single_value_shape() {
        let event = alb_event();
        let attrs =
            extract_header_attributes(resolve_headers(&event), "content-type", http_request_header);
        assert_eq!(
            get_str(&attrs, &http_request_header("content-type")),
            Some("application/json")
        );
    }

    /// ALB responses use the same `statusCode` field as API Gateway proxy
    /// integrations, plus an optional `statusDescription` that has no semconv
    /// attribute and is ignored.
    #[test]
    fn alb_response_status_code_and_multi_value_headers() {
        let response = serde_json::json!({
            "statusCode": 201,
            "statusDescription": "201 Created",
            "isBase64Encoded": false,
            "multiValueHeaders": {"content-type": ["application/json"]},
            "body": "{}"
        });
        assert_eq!(
            get_int(
                std::slice::from_ref(&extract_response_status_code_attribute(&response).unwrap()),
                HTTP_RESPONSE_STATUS_CODE
            ),
            Some(201)
        );
        let attrs = extract_header_attributes(
            resolve_headers(&response),
            "content-type",
            http_response_header,
        );
        assert_eq!(
            get_str(&attrs, &http_response_header("content-type")),
            Some("application/json")
        );
    }
}

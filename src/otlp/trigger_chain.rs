use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};

use crate::otlp::attributes::*;

const MAX_CHAIN_DEPTH: usize = 5;

/// A single hop in the trigger chain.
pub struct TriggerHop {
    pub trigger_type: String,
    pub arn: Option<String>,
    pub name: Option<String>,
    pub timestamp: Option<String>,
}

/// Result of trigger chain extraction.
pub struct TriggerChainResult {
    pub hops: Vec<TriggerHop>,
    pub truncated: bool,
}

/// Extracts a trigger chain from a Lambda event payload.
/// Returns hops ordered outermost (origin) to innermost (immediate trigger),
/// and a flag indicating if the chain was truncated due to depth limit.
pub fn extract_trigger_chain(event_payload: &str) -> TriggerChainResult {
    let json_val: serde_json::Value = match serde_json::from_str(event_payload) {
        Ok(v) => v,
        Err(_) => {
            return TriggerChainResult {
                hops: Vec::new(),
                truncated: false,
            }
        }
    };

    if let Some(records) = json_val.get("Records").and_then(|v| v.as_array()) {
        if let Some(first) = records.first() {
            return extract_from_record(first);
        }
    }

    // EventBridge: no Records array, has both "source" and "detail-type"
    if json_val.get("source").and_then(|v| v.as_str()).is_some()
        && json_val
            .get("detail-type")
            .and_then(|v| v.as_str())
            .is_some()
    {
        return TriggerChainResult {
            hops: vec![make_eventbridge_hop(&json_val)],
            truncated: false,
        };
    }

    // Kafka / MSK: eventSource at top level, records (lowercase) as map
    let top_event_source = json_val.get("eventSource").and_then(|v| v.as_str());
    if top_event_source == Some("aws:kafka") || top_event_source == Some("SelfManagedKafka") {
        return extract_kafka_chain(&json_val);
    }

    // API Gateway: has "requestContext"
    if json_val.get("requestContext").is_some() {
        return extract_api_gateway_chain(&json_val);
    }

    TriggerChainResult {
        hops: Vec::new(),
        truncated: false,
    }
}

fn extract_from_record(record: &serde_json::Value) -> TriggerChainResult {
    let event_source = record
        .get("eventSource")
        .or_else(|| record.get("EventSource"))
        .and_then(|v| v.as_str());

    match event_source {
        Some("aws:sqs") => extract_sqs_chain(record),
        Some("aws:sns") => extract_sns_chain(record),
        Some("aws:kinesis") => extract_kinesis_chain(record),
        Some("aws:dynamodb") => extract_dynamodb_chain(record),
        Some("aws:s3") => extract_s3_chain(record),
        _ => TriggerChainResult {
            hops: Vec::new(),
            truncated: false,
        },
    }
}

// ── SQS ──────────────────────────────────────────────────────────────

fn extract_sqs_chain(record: &serde_json::Value) -> TriggerChainResult {
    let sqs_hop = make_sqs_hop(record);

    if let Some(body_str) = record.get("body").and_then(|v| v.as_str()) {
        let (mut inner_hops, truncated) = try_parse_inner_event(body_str, 1);
        if !inner_hops.is_empty() {
            inner_hops.push(sqs_hop);
            return TriggerChainResult {
                hops: inner_hops,
                truncated,
            };
        }
    }

    TriggerChainResult {
        hops: vec![sqs_hop],
        truncated: false,
    }
}

fn make_sqs_hop(record: &serde_json::Value) -> TriggerHop {
    let arn = record
        .get("eventSourceARN")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = arn.as_deref().and_then(extract_name_from_arn);

    TriggerHop {
        trigger_type: "aws:sqs".to_string(),
        arn,
        name,
        timestamp: None,
    }
}

// ── SNS ──────────────────────────────────────────────────────────────

fn extract_sns_chain(record: &serde_json::Value) -> TriggerChainResult {
    let sns = match record.get("Sns") {
        Some(s) => s,
        None => {
            return TriggerChainResult {
                hops: Vec::new(),
                truncated: false,
            }
        }
    };

    let sns_hop = make_sns_hop(sns);

    // Check if the SNS Message contains a nested event
    if let Some(message_str) = sns.get("Message").and_then(|v| v.as_str()) {
        let (mut inner_hops, truncated) = try_parse_inner_event(message_str, 1);
        if !inner_hops.is_empty() {
            inner_hops.push(sns_hop);
            return TriggerChainResult {
                hops: inner_hops,
                truncated,
            };
        }
    }

    TriggerChainResult {
        hops: vec![sns_hop],
        truncated: false,
    }
}

fn make_sns_hop(sns: &serde_json::Value) -> TriggerHop {
    let arn = sns
        .get("TopicArn")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = arn.as_deref().and_then(extract_name_from_arn);
    let timestamp = sns
        .get("Timestamp")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    TriggerHop {
        trigger_type: "aws:sns".to_string(),
        arn,
        name,
        timestamp,
    }
}

// ── Recursive inner event parsing ────────────────────────────────────

/// Tries to parse a string as a known inner event type.
/// Returns (hops, truncated). Hops are in origin-first order.
fn try_parse_inner_event(body: &str, depth: usize) -> (Vec<TriggerHop>, bool) {
    if depth >= MAX_CHAIN_DEPTH {
        return (Vec::new(), true);
    }

    let json_val: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (Vec::new(), false),
    };

    // SNS Notification embedded in SQS body
    if json_val.get("Type").and_then(|v| v.as_str()) == Some("Notification") {
        if let Some(topic_arn) = json_val.get("TopicArn").and_then(|v| v.as_str()) {
            let sns_hop = TriggerHop {
                trigger_type: "aws:sns".to_string(),
                arn: Some(topic_arn.to_string()),
                name: extract_name_from_arn(topic_arn),
                timestamp: json_val
                    .get("Timestamp")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            };

            // Recurse into the SNS Message
            if let Some(message_str) = json_val.get("Message").and_then(|v| v.as_str()) {
                let (mut inner_hops, truncated) = try_parse_inner_event(message_str, depth + 1);
                if !inner_hops.is_empty() {
                    inner_hops.push(sns_hop);
                    return (inner_hops, truncated);
                }
                // Propagate truncation even if inner hops are empty
                // (depth limit was hit before any event could be parsed)
                if truncated {
                    return (vec![sns_hop], true);
                }
            }

            return (vec![sns_hop], false);
        }
    }

    // EventBridge event
    if json_val.get("source").and_then(|v| v.as_str()).is_some()
        && json_val
            .get("detail-type")
            .and_then(|v| v.as_str())
            .is_some()
    {
        return (vec![make_eventbridge_hop(&json_val)], false);
    }

    // S3 event (has Records with eventSource: aws:s3)
    if let Some(records) = json_val.get("Records").and_then(|v| v.as_array()) {
        if let Some(first) = records.first() {
            if first.get("eventSource").and_then(|v| v.as_str()) == Some("aws:s3") {
                if let Some(s3) = first.get("s3") {
                    let bucket = s3.get("bucket");
                    let arn = bucket
                        .and_then(|b| b.get("arn"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let name = bucket
                        .and_then(|b| b.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    return (
                        vec![TriggerHop {
                            trigger_type: "aws:s3".to_string(),
                            arn,
                            name,
                            timestamp: None,
                        }],
                        false,
                    );
                }
            }
        }
    }

    (Vec::new(), false)
}

// ── Kinesis ──────────────────────────────────────────────────────────

fn extract_kinesis_chain(record: &serde_json::Value) -> TriggerChainResult {
    let arn = record
        .get("eventSourceARN")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = arn.as_deref().and_then(extract_name_from_arn);

    TriggerChainResult {
        hops: vec![TriggerHop {
            trigger_type: "aws:kinesis".to_string(),
            arn,
            name,
            timestamp: None,
        }],
        truncated: false,
    }
}

// ── DynamoDB ─────────────────────────────────────────────────────────

fn extract_dynamodb_chain(record: &serde_json::Value) -> TriggerChainResult {
    let arn = record
        .get("eventSourceARN")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = arn.as_deref().and_then(extract_name_from_arn);

    TriggerChainResult {
        hops: vec![TriggerHop {
            trigger_type: "aws:dynamodb".to_string(),
            arn,
            name,
            timestamp: None,
        }],
        truncated: false,
    }
}

// ── S3 ───────────────────────────────────────────────────────────────

fn extract_s3_chain(record: &serde_json::Value) -> TriggerChainResult {
    let s3 = match record.get("s3") {
        Some(s) => s,
        None => {
            return TriggerChainResult {
                hops: Vec::new(),
                truncated: false,
            }
        }
    };

    let bucket = s3.get("bucket");
    let arn = bucket
        .and_then(|b| b.get("arn"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = bucket
        .and_then(|b| b.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    TriggerChainResult {
        hops: vec![TriggerHop {
            trigger_type: "aws:s3".to_string(),
            arn,
            name,
            timestamp: None,
        }],
        truncated: false,
    }
}

// ── EventBridge ──────────────────────────────────────────────────────

fn make_eventbridge_hop(json_val: &serde_json::Value) -> TriggerHop {
    let name = json_val
        .get("source")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let timestamp = json_val
        .get("time")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    TriggerHop {
        trigger_type: "aws:event_bridge".to_string(),
        arn: None,
        name,
        timestamp,
    }
}

// ── Kafka / MSK ──────────────────────────────────────────────────────

fn extract_kafka_chain(json_val: &serde_json::Value) -> TriggerChainResult {
    let event_source = json_val
        .get("eventSource")
        .and_then(|v| v.as_str())
        .unwrap_or("aws:kafka");

    // MSK has eventSourceArn, self-managed does not
    let arn = json_val
        .get("eventSourceArn")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract topic name from the first key in the records map (format: "topic-partition")
    let name = json_val
        .get("records")
        .and_then(|v| v.as_object())
        .and_then(|map| map.keys().next())
        .and_then(|key| {
            // Key format is "topic-partition" (e.g. "mytopic-0")
            // But topic names can contain hyphens, so find the last hyphen
            key.rfind('-').map(|pos| key[..pos].to_string())
        });

    let trigger_type = match event_source {
        "SelfManagedKafka" => "aws:kafka:self_managed",
        _ => "aws:kafka",
    };

    TriggerChainResult {
        hops: vec![TriggerHop {
            trigger_type: trigger_type.to_string(),
            arn,
            name,
            timestamp: None,
        }],
        truncated: false,
    }
}

// ── API Gateway ──────────────────────────────────────────────────────

fn extract_api_gateway_chain(json_val: &serde_json::Value) -> TriggerChainResult {
    let rc = match json_val.get("requestContext") {
        Some(rc) => rc,
        None => {
            return TriggerChainResult {
                hops: Vec::new(),
                truncated: false,
            }
        }
    };

    let is_v2 = json_val.get("version").and_then(|v| v.as_str()) == Some("2.0");

    let (method, path) = if is_v2 {
        let http = rc.get("http");
        (
            http.and_then(|h| h.get("method")).and_then(|v| v.as_str()),
            http.and_then(|h| h.get("path")).and_then(|v| v.as_str()),
        )
    } else {
        (
            rc.get("httpMethod").and_then(|v| v.as_str()),
            rc.get("path").and_then(|v| v.as_str()),
        )
    };

    let name = match (method, path) {
        (Some(m), Some(p)) => Some(format!("{} {}", m, p)),
        (None, Some(p)) => Some(p.to_string()),
        (Some(m), None) => Some(m.to_string()),
        (None, None) => None,
    };

    let arn = {
        let api_id = rc.get("apiId").and_then(|v| v.as_str());
        let account_id = rc.get("accountId").and_then(|v| v.as_str());
        let stage = rc.get("stage").and_then(|v| v.as_str());
        let domain = rc.get("domainName").and_then(|v| v.as_str());

        let region = domain.and_then(|d| {
            let parts: Vec<&str> = d.split('.').collect();
            if parts.len() >= 4 && parts[1] == "execute-api" {
                Some(parts[2])
            } else {
                None
            }
        });

        match (api_id, account_id, region, stage, method, path) {
            (Some(api), Some(acct), Some(reg), Some(stg), Some(m), Some(p)) => Some(format!(
                "arn:aws:execute-api:{}:{}:{}/{}/{}{}",
                reg, acct, api, stg, m, p
            )),
            _ => None,
        }
    };

    TriggerChainResult {
        hops: vec![TriggerHop {
            trigger_type: "aws:api_gateway".to_string(),
            arn,
            name,
            timestamp: None,
        }],
        truncated: false,
    }
}

// ── Attribute conversion ─────────────────────────────────────────────

/// Converts a trigger chain result into OTel span attributes.
pub fn trigger_chain_to_attributes(result: &TriggerChainResult) -> Vec<KeyValue> {
    if result.hops.is_empty() {
        return Vec::new();
    }

    let mut attributes = Vec::new();

    attributes.push(KeyValue {
        key: DASH0_TRIGGER_CHAIN_DEPTH.to_string(),
        value: Some(AnyValue {
            value: Some(Value::IntValue(result.hops.len() as i64)),
        }),
    });

    if result.truncated {
        attributes.push(KeyValue {
            key: DASH0_TRIGGER_CHAIN_TRUNCATED.to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(true)),
            }),
        });
    }

    for (i, hop) in result.hops.iter().enumerate() {
        attributes.push(KeyValue {
            key: dash0_trigger_chain_type(i),
            value: Some(AnyValue {
                value: Some(Value::StringValue(hop.trigger_type.clone())),
            }),
        });

        if let Some(arn) = &hop.arn {
            attributes.push(KeyValue {
                key: dash0_trigger_chain_arn(i),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(arn.clone())),
                }),
            });
        }

        if let Some(name) = &hop.name {
            attributes.push(KeyValue {
                key: dash0_trigger_chain_name(i),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(name.clone())),
                }),
            });
        }

        if let Some(timestamp) = &hop.timestamp {
            attributes.push(KeyValue {
                key: dash0_trigger_chain_timestamp(i),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(timestamp.clone())),
                }),
            });
        }
    }

    attributes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_attr<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
        attrs.iter().find_map(|kv| {
            if kv.key == key {
                if let Some(AnyValue {
                    value: Some(Value::StringValue(v)),
                }) = &kv.value
                {
                    return Some(v.as_str());
                }
            }
            None
        })
    }

    fn get_int_attr(attrs: &[KeyValue], key: &str) -> Option<i64> {
        attrs.iter().find_map(|kv| {
            if kv.key == key {
                if let Some(AnyValue {
                    value: Some(Value::IntValue(v)),
                }) = &kv.value
                {
                    return Some(*v);
                }
            }
            None
        })
    }

    fn get_bool_attr(attrs: &[KeyValue], key: &str) -> Option<bool> {
        attrs.iter().find_map(|kv| {
            if kv.key == key {
                if let Some(AnyValue {
                    value: Some(Value::BoolValue(v)),
                }) = &kv.value
                {
                    return Some(*v);
                }
            }
            None
        })
    }

    // ── Direct triggers (depth = 1) ──────────────────────────────────

    #[test]
    fn sqs_direct_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "eventSourceARN": "arn:aws:sqs:us-east-1:123456789:order-queue",
                "messageId": "msg-123",
                "body": "hello"
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert!(!result.truncated);
        assert_eq!(result.hops[0].trigger_type, "aws:sqs");
        assert_eq!(
            result.hops[0].arn.as_deref(),
            Some("arn:aws:sqs:us-east-1:123456789:order-queue")
        );
        assert_eq!(result.hops[0].name.as_deref(), Some("order-queue"));

        let attrs = trigger_chain_to_attributes(&result);
        assert_eq!(get_int_attr(&attrs, "dash0.trigger.chain.depth"), Some(1));
        assert!(get_bool_attr(&attrs, "dash0.trigger.chain.truncated").is_none());
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.0.type"),
            Some("aws:sqs")
        );
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.0.arn"),
            Some("arn:aws:sqs:us-east-1:123456789:order-queue")
        );
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.0.name"),
            Some("order-queue")
        );
    }

    #[test]
    fn sqs_without_arn() {
        let result = extract_trigger_chain(
            r#"{"Records": [{"eventSource": "aws:sqs", "body": "hello"}]}"#,
        );
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:sqs");
        assert!(result.hops[0].arn.is_none());
        assert!(result.hops[0].name.is_none());
    }

    #[test]
    fn sns_direct_trigger() {
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns",
                "Sns": {
                    "TopicArn": "arn:aws:sns:us-east-1:123456789:order-topic",
                    "MessageId": "msg-123",
                    "Timestamp": "2026-03-31T10:00:00.000Z",
                    "Message": "hello"
                }
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:sns");
        assert_eq!(
            result.hops[0].arn.as_deref(),
            Some("arn:aws:sns:us-east-1:123456789:order-topic")
        );
        assert_eq!(result.hops[0].name.as_deref(), Some("order-topic"));
        assert_eq!(
            result.hops[0].timestamp.as_deref(),
            Some("2026-03-31T10:00:00.000Z")
        );
    }

    #[test]
    fn sns_without_sns_object() {
        let result =
            extract_trigger_chain(r#"{"Records": [{"EventSource": "aws:sns"}]}"#);
        assert!(result.hops.is_empty());
    }

    #[test]
    fn kinesis_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:kinesis",
                "eventSourceARN": "arn:aws:kinesis:us-east-1:123456789:stream/order-stream",
                "kinesis": {"data": "aGVsbG8=", "sequenceNumber": "123"}
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:kinesis");
        assert_eq!(result.hops[0].name.as_deref(), Some("order-stream"));
    }

    #[test]
    fn dynamodb_streams_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:dynamodb",
                "eventSourceARN": "arn:aws:dynamodb:us-east-1:123456789:table/orders/stream/2025-01-01T00:00:00.000",
                "eventName": "INSERT",
                "dynamodb": {"Keys": {"id": {"S": "123"}}}
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:dynamodb");
        assert_eq!(result.hops[0].name.as_deref(), Some("orders"));
    }

    #[test]
    fn s3_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:s3",
                "eventName": "ObjectCreated:Put",
                "s3": {
                    "bucket": {"name": "my-bucket", "arn": "arn:aws:s3:::my-bucket"},
                    "object": {"key": "uploads/file.json"}
                }
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:s3");
        assert_eq!(result.hops[0].arn.as_deref(), Some("arn:aws:s3:::my-bucket"));
        assert_eq!(result.hops[0].name.as_deref(), Some("my-bucket"));
    }

    #[test]
    fn s3_without_s3_object() {
        let result =
            extract_trigger_chain(r#"{"Records": [{"eventSource": "aws:s3"}]}"#);
        assert!(result.hops.is_empty());
    }

    #[test]
    fn eventbridge_trigger() {
        let payload = r#"{
            "version": "0",
            "source": "my-app.orders",
            "detail-type": "OrderCreated",
            "id": "evt-123",
            "time": "2026-03-31T12:20:00Z",
            "detail": {"orderId": "456"}
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:event_bridge");
        assert_eq!(result.hops[0].name.as_deref(), Some("my-app.orders"));
        assert_eq!(
            result.hops[0].timestamp.as_deref(),
            Some("2026-03-31T12:20:00Z")
        );
    }

    #[test]
    fn eventbridge_requires_both_source_and_detail_type() {
        assert!(extract_trigger_chain(r#"{"source": "my-app"}"#).hops.is_empty());
        assert!(
            extract_trigger_chain(r#"{"detail-type": "OrderCreated"}"#)
                .hops
                .is_empty()
        );
    }

    #[test]
    fn api_gateway_v1_trigger() {
        let payload = r#"{
            "requestContext": {
                "httpMethod": "POST", "path": "/orders",
                "apiId": "abc123", "accountId": "123456789",
                "stage": "prod",
                "domainName": "abc123.execute-api.us-east-1.amazonaws.com"
            },
            "body": "hello"
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:api_gateway");
        assert_eq!(result.hops[0].name.as_deref(), Some("POST /orders"));
        assert_eq!(
            result.hops[0].arn.as_deref(),
            Some("arn:aws:execute-api:us-east-1:123456789:abc123/prod/POST/orders")
        );
    }

    #[test]
    fn api_gateway_v2_trigger() {
        let payload = r#"{
            "version": "2.0",
            "requestContext": {
                "http": {"method": "GET", "path": "/users"},
                "apiId": "xyz789", "accountId": "123456789",
                "stage": "$default",
                "domainName": "xyz789.execute-api.eu-west-1.amazonaws.com"
            }
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:api_gateway");
        assert_eq!(result.hops[0].name.as_deref(), Some("GET /users"));
        assert_eq!(
            result.hops[0].arn.as_deref(),
            Some("arn:aws:execute-api:eu-west-1:123456789:xyz789/$default/GET/users")
        );
    }

    #[test]
    fn api_gateway_without_enough_info_for_arn() {
        let payload = r#"{
            "requestContext": {"httpMethod": "POST", "path": "/orders"},
            "body": "hello"
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].name.as_deref(), Some("POST /orders"));
        assert!(result.hops[0].arn.is_none());
    }

    // ── Edge cases ───────────────────────────────────────────────────

    #[test]
    fn empty_records() {
        assert!(extract_trigger_chain(r#"{"Records": []}"#).hops.is_empty());
    }

    #[test]
    fn no_records() {
        assert!(extract_trigger_chain(r#"{"foo": "bar"}"#).hops.is_empty());
    }

    #[test]
    fn invalid_json() {
        assert!(extract_trigger_chain("not json").hops.is_empty());
    }

    #[test]
    fn unknown_event_source() {
        let result = extract_trigger_chain(
            r#"{"Records": [{"eventSource": "aws:unknown", "body": "hello"}]}"#,
        );
        assert!(result.hops.is_empty());
    }

    #[test]
    fn empty_chain_produces_no_attributes() {
        let result = TriggerChainResult {
            hops: Vec::new(),
            truncated: false,
        };
        assert!(trigger_chain_to_attributes(&result).is_empty());
    }

    // ── Nested triggers (depth = 2) ──────────────────────────────────

    #[test]
    fn sns_via_sqs_nested_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "eventSourceARN": "arn:aws:sqs:us-east-1:123456789:order-queue",
                "messageId": "sqs-msg-456",
                "body": "{\"Type\":\"Notification\",\"TopicArn\":\"arn:aws:sns:us-east-1:123456789:order-topic\",\"MessageId\":\"sns-msg-123\",\"Timestamp\":\"2026-03-31T10:00:00.000Z\",\"Message\":\"hello\"}"
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 2);
        assert!(!result.truncated);

        assert_eq!(result.hops[0].trigger_type, "aws:sns");
        assert_eq!(
            result.hops[0].arn.as_deref(),
            Some("arn:aws:sns:us-east-1:123456789:order-topic")
        );
        assert_eq!(result.hops[0].name.as_deref(), Some("order-topic"));
        assert_eq!(
            result.hops[0].timestamp.as_deref(),
            Some("2026-03-31T10:00:00.000Z")
        );

        assert_eq!(result.hops[1].trigger_type, "aws:sqs");
        assert_eq!(
            result.hops[1].arn.as_deref(),
            Some("arn:aws:sqs:us-east-1:123456789:order-queue")
        );
        assert_eq!(result.hops[1].name.as_deref(), Some("order-queue"));

        let attrs = trigger_chain_to_attributes(&result);
        assert_eq!(get_int_attr(&attrs, "dash0.trigger.chain.depth"), Some(2));
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.0.type"),
            Some("aws:sns")
        );
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.1.type"),
            Some("aws:sqs")
        );
    }

    #[test]
    fn eventbridge_via_sqs_nested_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "eventSourceARN": "arn:aws:sqs:us-east-1:123456789:order-queue",
                "body": "{\"source\":\"my-app.orders\",\"detail-type\":\"OrderCreated\",\"time\":\"2026-03-31T12:20:00Z\",\"detail\":{\"orderId\":\"456\"}}"
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 2);

        assert_eq!(result.hops[0].trigger_type, "aws:event_bridge");
        assert_eq!(result.hops[0].name.as_deref(), Some("my-app.orders"));
        assert_eq!(
            result.hops[0].timestamp.as_deref(),
            Some("2026-03-31T12:20:00Z")
        );

        assert_eq!(result.hops[1].trigger_type, "aws:sqs");
        assert_eq!(result.hops[1].name.as_deref(), Some("order-queue"));
    }

    #[test]
    fn s3_via_sqs_nested_trigger() {
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "eventSourceARN": "arn:aws:sqs:us-east-1:123456789:notif-queue",
                "body": "{\"Records\":[{\"eventSource\":\"aws:s3\",\"s3\":{\"bucket\":{\"name\":\"my-bucket\",\"arn\":\"arn:aws:s3:::my-bucket\"},\"object\":{\"key\":\"file.json\"}}}]}"
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 2);

        assert_eq!(result.hops[0].trigger_type, "aws:s3");
        assert_eq!(result.hops[0].name.as_deref(), Some("my-bucket"));

        assert_eq!(result.hops[1].trigger_type, "aws:sqs");
        assert_eq!(result.hops[1].name.as_deref(), Some("notif-queue"));
    }

    #[test]
    fn s3_via_sns_direct_to_lambda() {
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns",
                "Sns": {
                    "TopicArn": "arn:aws:sns:us-east-1:123456789:s3-events",
                    "Timestamp": "2026-03-31T10:00:00.000Z",
                    "Message": "{\"Records\":[{\"eventSource\":\"aws:s3\",\"s3\":{\"bucket\":{\"name\":\"my-bucket\",\"arn\":\"arn:aws:s3:::my-bucket\"},\"object\":{\"key\":\"file.json\"}}}]}"
                }
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 2);

        assert_eq!(result.hops[0].trigger_type, "aws:s3");
        assert_eq!(result.hops[0].name.as_deref(), Some("my-bucket"));

        assert_eq!(result.hops[1].trigger_type, "aws:sns");
        assert_eq!(result.hops[1].name.as_deref(), Some("s3-events"));
    }

    // ── Nested triggers (depth = 3) ──────────────────────────────────

    #[test]
    fn s3_via_sns_via_sqs_nested_trigger() {
        // S3 event inside SNS Message inside SQS body
        let s3_event = r#"{"Records":[{"eventSource":"aws:s3","s3":{"bucket":{"name":"my-bucket","arn":"arn:aws:s3:::my-bucket"},"object":{"key":"file.json"}}}]}"#;
        let sns_notification = format!(
            r#"{{"Type":"Notification","TopicArn":"arn:aws:sns:us-east-1:123456789:s3-events","Timestamp":"2026-03-31T10:00:00.000Z","Message":{}}}"#,
            serde_json::to_string(s3_event).unwrap()
        );
        let payload = format!(
            r#"{{"Records":[{{"eventSource":"aws:sqs","eventSourceARN":"arn:aws:sqs:us-east-1:123456789:notif-queue","body":{}}}]}}"#,
            serde_json::to_string(&sns_notification).unwrap()
        );

        let result = extract_trigger_chain(&payload);
        assert_eq!(result.hops.len(), 3);
        assert!(!result.truncated);

        assert_eq!(result.hops[0].trigger_type, "aws:s3");
        assert_eq!(result.hops[0].name.as_deref(), Some("my-bucket"));

        assert_eq!(result.hops[1].trigger_type, "aws:sns");
        assert_eq!(result.hops[1].name.as_deref(), Some("s3-events"));

        assert_eq!(result.hops[2].trigger_type, "aws:sqs");
        assert_eq!(result.hops[2].name.as_deref(), Some("notif-queue"));

        let attrs = trigger_chain_to_attributes(&result);
        assert_eq!(get_int_attr(&attrs, "dash0.trigger.chain.depth"), Some(3));
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.0.type"),
            Some("aws:s3")
        );
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.1.type"),
            Some("aws:sns")
        );
        assert_eq!(
            get_attr(&attrs, "dash0.trigger.chain.2.type"),
            Some("aws:sqs")
        );
    }

    // ── Truncation ───────────────────────────────────────────────────

    #[test]
    fn truncation_flag_set_when_depth_limit_reached() {
        // Build a deeply nested SNS chain that exceeds MAX_CHAIN_DEPTH.
        // SQS body parsing starts at depth=1. Each nested SNS Notification adds +1.
        // We need enough levels so that try_parse_inner_event is called with depth >= MAX_CHAIN_DEPTH.
        // That means we need MAX_CHAIN_DEPTH nested SNS Notifications inside the SQS body.
        let mut inner = r#"{"Type":"Notification","TopicArn":"arn:aws:sns:us-east-1:123:deepest","Timestamp":"2026-01-01T00:00:00Z","Message":"leaf"}"#.to_string();
        for i in 0..MAX_CHAIN_DEPTH + 1 {
            inner = format!(
                r#"{{"Type":"Notification","TopicArn":"arn:aws:sns:us-east-1:123:level-{}","Timestamp":"2026-01-01T00:00:00Z","Message":{}}}"#,
                i,
                serde_json::to_string(&inner).unwrap()
            );
        }
        let payload = format!(
            r#"{{"Records":[{{"eventSource":"aws:sqs","eventSourceARN":"arn:aws:sqs:us-east-1:123:q","body":{}}}]}}"#,
            serde_json::to_string(&inner).unwrap()
        );

        let result = extract_trigger_chain(&payload);
        assert!(result.truncated);
        assert!(result.hops.len() <= MAX_CHAIN_DEPTH + 1);

        let attrs = trigger_chain_to_attributes(&result);
        assert_eq!(
            get_bool_attr(&attrs, "dash0.trigger.chain.truncated"),
            Some(true)
        );
    }

    // ── Kafka / MSK ────────────────────────────────────────────────

    #[test]
    fn msk_trigger() {
        let payload = r#"{
            "eventSource": "aws:kafka",
            "eventSourceArn": "arn:aws:kafka:us-east-1:123456789:cluster/my-cluster/abc-123",
            "bootstrapServers": "b-1.my-cluster.kafka.us-east-1.amazonaws.com:9092",
            "records": {
                "order-events-0": [{
                    "topic": "order-events",
                    "partition": 0,
                    "offset": 15,
                    "timestamp": 1545084650987,
                    "value": "aGVsbG8="
                }]
            }
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:kafka");
        assert_eq!(
            result.hops[0].arn.as_deref(),
            Some("arn:aws:kafka:us-east-1:123456789:cluster/my-cluster/abc-123")
        );
        assert_eq!(result.hops[0].name.as_deref(), Some("order-events"));
    }

    #[test]
    fn self_managed_kafka_trigger() {
        let payload = r#"{
            "eventSource": "SelfManagedKafka",
            "bootstrapServers": "my-kafka:9092",
            "records": {
                "my-topic-0": [{
                    "topic": "my-topic",
                    "partition": 0,
                    "offset": 0,
                    "value": "aGVsbG8="
                }]
            }
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:kafka:self_managed");
        assert!(result.hops[0].arn.is_none());
        assert_eq!(result.hops[0].name.as_deref(), Some("my-topic"));
    }

    #[test]
    fn kafka_topic_name_with_hyphens() {
        let payload = r#"{
            "eventSource": "aws:kafka",
            "records": {
                "my-hyphenated-topic-name-3": [{
                    "topic": "my-hyphenated-topic-name",
                    "partition": 3,
                    "offset": 0,
                    "value": "aGVsbG8="
                }]
            }
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops[0].name.as_deref(), Some("my-hyphenated-topic-name"));
    }

    #[test]
    fn kafka_empty_records_map() {
        let payload = r#"{
            "eventSource": "aws:kafka",
            "records": {}
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:kafka");
        assert!(result.hops[0].name.is_none());
    }

    // ── SNS fanout (SNS -> SNS -> SQS) ──────────────────────────────

    #[test]
    fn sns_fanout_via_sqs() {
        // SNS -> SNS -> SQS -> Lambda
        let inner_sns = r#"{"Type":"Notification","TopicArn":"arn:aws:sns:us-east-1:123:origin-topic","Timestamp":"2026-03-31T09:00:00.000Z","Message":"original payload"}"#;
        let outer_sns = format!(
            r#"{{"Type":"Notification","TopicArn":"arn:aws:sns:us-east-1:123:fanout-topic","Timestamp":"2026-03-31T09:00:01.000Z","Message":{}}}"#,
            serde_json::to_string(inner_sns).unwrap()
        );
        let payload = format!(
            r#"{{"Records":[{{"eventSource":"aws:sqs","eventSourceARN":"arn:aws:sqs:us-east-1:123:my-queue","body":{}}}]}}"#,
            serde_json::to_string(&outer_sns).unwrap()
        );

        let result = extract_trigger_chain(&payload);
        assert_eq!(result.hops.len(), 3);
        assert!(!result.truncated);

        assert_eq!(result.hops[0].trigger_type, "aws:sns");
        assert_eq!(result.hops[0].name.as_deref(), Some("origin-topic"));

        assert_eq!(result.hops[1].trigger_type, "aws:sns");
        assert_eq!(result.hops[1].name.as_deref(), Some("fanout-topic"));

        assert_eq!(result.hops[2].trigger_type, "aws:sqs");
        assert_eq!(result.hops[2].name.as_deref(), Some("my-queue"));
    }

    // ── Additional edge cases ────────────────────────────────────────

    #[test]
    fn multiple_records_only_first_is_used() {
        let payload = r#"{
            "Records": [
                {
                    "eventSource": "aws:sqs",
                    "eventSourceARN": "arn:aws:sqs:us-east-1:123:first-queue",
                    "body": "hello"
                },
                {
                    "eventSource": "aws:sqs",
                    "eventSourceARN": "arn:aws:sqs:us-east-1:123:second-queue",
                    "body": "world"
                }
            ]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].name.as_deref(), Some("first-queue"));
    }

    #[test]
    fn first_record_has_no_event_source() {
        let payload = r#"{
            "Records": [{"body": "hello"}]
        }"#;

        let result = extract_trigger_chain(payload);
        assert!(result.hops.is_empty());
    }

    // ── Fallback cases ───────────────────────────────────────────────

    #[test]
    fn sqs_body_is_json_but_not_recognized_event() {
        let result = extract_trigger_chain(
            r#"{"Records": [{"eventSource": "aws:sqs", "eventSourceARN": "arn:aws:sqs:us-east-1:123:q", "body": "{\"foo\": \"bar\"}"}]}"#,
        );
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:sqs");
    }

    #[test]
    fn sqs_body_is_invalid_json() {
        let result = extract_trigger_chain(
            r#"{"Records": [{"eventSource": "aws:sqs", "eventSourceARN": "arn:aws:sqs:us-east-1:123:q", "body": "just a plain string"}]}"#,
        );
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:sqs");
    }

    #[test]
    fn sqs_body_is_notification_but_no_topic_arn() {
        let result = extract_trigger_chain(
            r#"{"Records": [{"eventSource": "aws:sqs", "eventSourceARN": "arn:aws:sqs:us-east-1:123:q", "body": "{\"Type\":\"Notification\",\"Message\":\"hello\"}"}]}"#,
        );
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:sqs");
    }

    #[test]
    fn sns_message_is_plain_text_no_nesting() {
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns",
                "Sns": {
                    "TopicArn": "arn:aws:sns:us-east-1:123456789:order-topic",
                    "Timestamp": "2026-03-31T10:00:00.000Z",
                    "Message": "just a plain text message"
                }
            }]
        }"#;

        let result = extract_trigger_chain(payload);
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].trigger_type, "aws:sns");
    }
}

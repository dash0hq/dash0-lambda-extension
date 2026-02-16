use base64::Engine;
use opentelemetry_proto::tonic::trace::v1::span::Link;

/// Extracts span links from SQS, SNS, EventBridge, and Kinesis event payloads.
/// For SQS: looks for Records[].messageAttributes["x-amzn-trace-id"].stringValue first,
///   then falls back to Records[].messageAttributes.traceparent.stringValue,
///   then tries SNS message embedded in SQS body
/// For SNS: looks for Records[].Sns.MessageAttributes.traceparent.Value
/// For EventBridge: looks for detail.traceparent
/// For Kinesis: base64-decodes Records[].kinesis.data, then looks for X-Amzn-Trace-Id first,
///   then falls back to traceparent
pub fn extract_span_links(event_payload: &str) -> Vec<Link> {
    if !crate::config::user::is_extract_span_links_in_consumer() {
        return Vec::new();
    }

    let json_val: serde_json::Value = match serde_json::from_str(event_payload) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    // Check for EventBridge event (has "detail" and "detail-type" fields, no "Records")
    if json_val.get("detail-type").is_some() {
        return extract_eventbridge_link(&json_val).into_iter().collect();
    }

    let records = match json_val.get("Records").and_then(|v| v.as_array()) {
        Some(r) => r,
        None => return Vec::new(),
    };

    let mut links = Vec::new();
    for record in records {
        if let Some(link) = extract_link_from_record(record) {
            links.push(link);
        }
    }
    links
}

fn extract_link_from_record(record: &serde_json::Value) -> Option<Link> {
    // Check for SQS event (lowercase eventSource)
    if record.get("eventSource").and_then(|v| v.as_str()) == Some("aws:sqs") {
        return extract_sqs_link(record);
    }

    // Check for SNS event (PascalCase EventSource)
    if record.get("EventSource").and_then(|v| v.as_str()) == Some("aws:sns") {
        return extract_sns_link(record);
    }

    // Check for Kinesis event
    if record.get("eventSource").and_then(|v| v.as_str()) == Some("aws:kinesis") {
        return extract_kinesis_link(record);
    }

    None
}

fn extract_sqs_link(record: &serde_json::Value) -> Option<Link> {
    let message_attrs = record.get("messageAttributes")?;

    // First try: x-amzn-trace-id in direct SQS messageAttributes
    if let Some(link) = extract_link_from_amzn_trace_id(message_attrs) {
        return Some(link);
    }

    // Second try: traceparent in direct SQS messageAttributes
    if let Some(link) = extract_link_from_traceparent_sqs(message_attrs) {
        return Some(link);
    }

    // Third try: SNS message embedded in SQS body (SNS → SQS → Lambda pattern)
    extract_link_from_sqs_body(record)
}

fn extract_link_from_amzn_trace_id(message_attrs: &serde_json::Value) -> Option<Link> {
    let amzn_trace_str = message_attrs
        .get("x-amzn-trace-id")
        .and_then(|tp| tp.get("stringValue"))
        .and_then(|sv| sv.as_str())?;

    let (trace_id, span_id) = parse_amzn_trace_id(amzn_trace_str)?;
    Some(Link {
        trace_id,
        span_id,
        ..Default::default()
    })
}

fn extract_link_from_traceparent_sqs(message_attrs: &serde_json::Value) -> Option<Link> {
    let traceparent = message_attrs
        .get("traceparent")
        .and_then(|tp| tp.get("stringValue"))
        .and_then(|sv| sv.as_str())?;

    let (trace_id, span_id) = parse_traceparent(traceparent)?;
    Some(Link {
        trace_id,
        span_id,
        ..Default::default()
    })
}

fn extract_link_from_sqs_body(record: &serde_json::Value) -> Option<Link> {
    let body_str = record.get("body").and_then(|b| b.as_str())?;
    let body_json: serde_json::Value = serde_json::from_str(body_str).ok()?;
    let sns_attrs = body_json.get("MessageAttributes")?;

    // Try x-amzn-trace-id first, then traceparent
    if let Some(amzn_val) = sns_attrs
        .get("x-amzn-trace-id")
        .and_then(|tp| tp.get("Value"))
        .and_then(|v| v.as_str())
    {
        if let Some((trace_id, span_id)) = parse_amzn_trace_id(amzn_val) {
            return Some(Link {
                trace_id,
                span_id,
                ..Default::default()
            });
        }
    }

    let traceparent = sns_attrs
        .get("traceparent")
        .and_then(|tp| tp.get("Value"))
        .and_then(|v| v.as_str())?;

    let (trace_id, span_id) = parse_traceparent(traceparent)?;
    Some(Link {
        trace_id,
        span_id,
        ..Default::default()
    })
}

fn extract_sns_link(record: &serde_json::Value) -> Option<Link> {
    let sns_attrs = record
        .get("Sns")
        .and_then(|sns| sns.get("MessageAttributes"))?;

    // Try x-amzn-trace-id first, then traceparent
    if let Some(amzn_val) = sns_attrs
        .get("x-amzn-trace-id")
        .and_then(|tp| tp.get("Value"))
        .and_then(|v| v.as_str())
    {
        if let Some((trace_id, span_id)) = parse_amzn_trace_id(amzn_val) {
            return Some(Link {
                trace_id,
                span_id,
                ..Default::default()
            });
        }
    }

    let traceparent = sns_attrs
        .get("traceparent")
        .and_then(|tp| tp.get("Value"))
        .and_then(|v| v.as_str())?;

    let (trace_id, span_id) = parse_traceparent(traceparent)?;
    Some(Link {
        trace_id,
        span_id,
        ..Default::default()
    })
}

fn extract_kinesis_link(record: &serde_json::Value) -> Option<Link> {
    let data_b64 = record
        .get("kinesis")
        .and_then(|k| k.get("data"))
        .and_then(|d| d.as_str())?;

    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .ok()?;
    let decoded_str = std::str::from_utf8(&decoded_bytes).ok()?;
    let data_json: serde_json::Value = serde_json::from_str(decoded_str).ok()?;

    // Try X-Amzn-Trace-Id first, then traceparent
    if let Some(amzn_val) = data_json
        .get("X-Amzn-Trace-Id")
        .and_then(|v| v.as_str())
    {
        if let Some((trace_id, span_id)) = parse_amzn_trace_id(amzn_val) {
            return Some(Link {
                trace_id,
                span_id,
                ..Default::default()
            });
        }
    }

    let traceparent = data_json.get("traceparent").and_then(|v| v.as_str())?;
    let (trace_id, span_id) = parse_traceparent(traceparent)?;
    Some(Link {
        trace_id,
        span_id,
        ..Default::default()
    })
}

fn extract_eventbridge_link(json_val: &serde_json::Value) -> Option<Link> {
    let traceparent = json_val
        .get("detail")
        .and_then(|d| d.get("traceparent"))
        .and_then(|tp| tp.as_str())?;

    let (trace_id, span_id) = parse_traceparent(traceparent)?;
    Some(Link {
        trace_id,
        span_id,
        ..Default::default()
    })
}

/// Parses a W3C traceparent header and returns (trace_id, span_id) as byte vectors.
/// Format: {version}-{trace_id}-{span_id}-{flags}
/// Example: 00-026d2b5d090c15f6423df90800000000-157c058e59db86fb-01
fn parse_traceparent(traceparent: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let parts: Vec<&str> = traceparent.split('-').collect();
    if parts.len() != 4 {
        return None;
    }

    let trace_id = hex::decode(parts[1]).ok()?;
    let span_id = hex::decode(parts[2]).ok()?;

    if trace_id.len() != 16 || span_id.len() != 8 {
        return None;
    }

    Some((trace_id, span_id))
}

/// Parses an X-Ray trace ID (x-amzn-trace-id) and returns (trace_id, span_id) as byte vectors.
/// Format: Root=1-{8hex}-{24hex};Parent={16hex};Sampled={0|1}
/// Example: Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1
/// The 16-byte trace_id is the concatenation of the two hex parts after "Root=1-".
/// The 8-byte span_id is the hex part after "Parent=".
fn parse_amzn_trace_id(value: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut root_hex: Option<String> = None;
    let mut parent_hex: Option<&str> = None;

    for part in value.split(';') {
        let part = part.trim();
        if let Some(root_val) = part.strip_prefix("Root=") {
            // Expected: 1-{8hex}-{24hex}
            let root_parts: Vec<&str> = root_val.splitn(3, '-').collect();
            if root_parts.len() == 3 && root_parts[0] == "1" {
                // Concatenate the two hex segments to form 32 hex chars = 16 bytes
                root_hex = Some(format!("{}{}", root_parts[1], root_parts[2]));
            }
        } else if let Some(parent_val) = part.strip_prefix("Parent=") {
            parent_hex = Some(parent_val);
        }
    }

    let trace_id = hex::decode(root_hex.as_deref()?).ok()?;
    let span_id = hex::decode(parent_hex?).ok()?;

    if trace_id.len() != 16 || span_id.len() != 8 {
        return None;
    }

    Some((trace_id, span_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serial_test::serial;

    // ── parse_traceparent ───────────────────────────────────────────

    #[test]
    fn parse_traceparent_valid() {
        let traceparent = "00-026d2b5d090c15f6423df90800000000-157c058e59db86fb-01";
        let (trace_id, span_id) = parse_traceparent(traceparent).expect("should parse");

        assert_eq!(trace_id.len(), 16);
        assert_eq!(span_id.len(), 8);
        assert_eq!(hex::encode(&trace_id), "026d2b5d090c15f6423df90800000000");
        assert_eq!(hex::encode(&span_id), "157c058e59db86fb");
    }

    #[test]
    fn parse_traceparent_invalid_format() {
        assert!(parse_traceparent("invalid").is_none());
        assert!(parse_traceparent("00-abc-def-01").is_none());
        assert!(parse_traceparent("").is_none());
    }

    // ── parse_amzn_trace_id ─────────────────────────────────────────

    #[test]
    fn parse_amzn_trace_id_valid() {
        let val = "Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1";
        let (trace_id, span_id) = parse_amzn_trace_id(val).expect("should parse");

        assert_eq!(trace_id.len(), 16);
        assert_eq!(span_id.len(), 8);
        assert_eq!(hex::encode(&trace_id), "698f814c7708a2b018bc2cc4726a6288");
        assert_eq!(hex::encode(&span_id), "f21a582b8b8134b9");
    }

    #[test]
    fn parse_amzn_trace_id_missing_parent() {
        let val = "Root=1-698f814c-7708a2b018bc2cc4726a6288;Sampled=1";
        assert!(parse_amzn_trace_id(val).is_none());
    }

    #[test]
    fn parse_amzn_trace_id_missing_root() {
        let val = "Parent=f21a582b8b8134b9;Sampled=1";
        assert!(parse_amzn_trace_id(val).is_none());
    }

    #[test]
    fn parse_amzn_trace_id_invalid_hex() {
        let val = "Root=1-zzzzzzzz-zzzzzzzzzzzzzzzzzzzzzzzz;Parent=f21a582b8b8134b9;Sampled=1";
        assert!(parse_amzn_trace_id(val).is_none());
    }

    #[test]
    fn parse_amzn_trace_id_empty() {
        assert!(parse_amzn_trace_id("").is_none());
    }

    // ── extract_span_links: SQS with x-amzn-trace-id ───────────────

    #[test]
    #[serial]
    fn extract_sqs_link_from_amzn_trace_id() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {
                    "x-amzn-trace-id": {
                        "stringValue": "Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1",
                        "dataType": "String"
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "698f814c7708a2b018bc2cc4726a6288"
        );
        assert_eq!(hex::encode(&links[0].span_id), "f21a582b8b8134b9");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_sqs_link_prefers_amzn_trace_id_over_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {
                    "x-amzn-trace-id": {
                        "stringValue": "Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1",
                        "dataType": "String"
                    },
                    "traceparent": {
                        "stringValue": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        // Should use x-amzn-trace-id, NOT traceparent
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "698f814c7708a2b018bc2cc4726a6288"
        );
        assert_eq!(hex::encode(&links[0].span_id), "f21a582b8b8134b9");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_sqs_link_falls_back_to_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {
                    "traceparent": {
                        "stringValue": "00-026d2b5d090c15f6423df90800000000-157c058e59db86fb-01"
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "026d2b5d090c15f6423df90800000000"
        );
        assert_eq!(hex::encode(&links[0].span_id), "157c058e59db86fb");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: SQS with multiple records ───────────────

    #[test]
    #[serial]
    fn extract_sqs_links_with_multiple_records() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [
                {
                    "eventSource": "aws:sqs",
                    "messageAttributes": {
                        "traceparent": {
                            "stringValue": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
                        }
                    }
                },
                {
                    "eventSource": "aws:sqs",
                    "messageAttributes": {
                        "traceparent": {
                            "stringValue": "00-11112222333344445555666677778888-5555666677778888-01"
                        }
                    }
                }
            ]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 2);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "aaaabbbbccccddddeeeeffffaaaabbbb"
        );
        assert_eq!(
            hex::encode(&links[1].trace_id),
            "11112222333344445555666677778888"
        );
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: SQS edge cases ──────────────────────────

    #[test]
    #[serial]
    fn extract_sqs_links_handles_missing_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {}
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: SNS ─────────────────────────────────────

    #[test]
    #[serial]
    fn extract_sns_links_with_valid_event() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns",
                "Sns": {
                    "MessageAttributes": {
                        "traceparent": {
                            "Type": "String",
                            "Value": "00-0e9448e94692132e3aa97f4300000000-e17b75c674b168ae-01"
                        }
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "0e9448e94692132e3aa97f4300000000"
        );
        assert_eq!(hex::encode(&links[0].span_id), "e17b75c674b168ae");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_sns_links_prefers_amzn_trace_id_over_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns",
                "Sns": {
                    "MessageAttributes": {
                        "x-amzn-trace-id": {
                            "Type": "String",
                            "Value": "Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1"
                        },
                        "traceparent": {
                            "Type": "String",
                            "Value": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
                        }
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "698f814c7708a2b018bc2cc4726a6288"
        );
        assert_eq!(hex::encode(&links[0].span_id), "f21a582b8b8134b9");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_sns_links_with_multiple_records() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [
                {
                    "EventSource": "aws:sns",
                    "Sns": {
                        "MessageAttributes": {
                            "traceparent": {
                                "Type": "String",
                                "Value": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
                            }
                        }
                    }
                },
                {
                    "EventSource": "aws:sns",
                    "Sns": {
                        "MessageAttributes": {
                            "traceparent": {
                                "Type": "String",
                                "Value": "00-11112222333344445555666677778888-5555666677778888-01"
                            }
                        }
                    }
                }
            ]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 2);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "aaaabbbbccccddddeeeeffffaaaabbbb"
        );
        assert_eq!(
            hex::encode(&links[1].trace_id),
            "11112222333344445555666677778888"
        );
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_sns_links_handles_missing_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns",
                "Sns": {
                    "MessageAttributes": {}
                }
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_sns_links_handles_missing_sns_object() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "EventSource": "aws:sns"
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: SNS via SQS ─────────────────────────────

    #[test]
    #[serial]
    fn extract_links_from_sns_via_sqs() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {},
                "body": "{\"Type\":\"Notification\",\"MessageId\":\"a5bc81a6\",\"MessageAttributes\":{\"traceparent\":{\"Type\":\"String\",\"Value\":\"00-547e30d6367841ef2fb1000600000000-d24fe80e627d602e-01\"}}}"
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "547e30d6367841ef2fb1000600000000"
        );
        assert_eq!(hex::encode(&links[0].span_id), "d24fe80e627d602e");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_from_sns_via_sqs_prefers_direct_message_attributes() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {
                    "traceparent": {
                        "stringValue": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
                    }
                },
                "body": "{\"MessageAttributes\":{\"traceparent\":{\"Type\":\"String\",\"Value\":\"00-11112222333344445555666677778888-5555666677778888-01\"}}}"
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "aaaabbbbccccddddeeeeffffaaaabbbb"
        );
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_from_sns_via_sqs_with_amzn_trace_id_in_body() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {},
                "body": "{\"Type\":\"Notification\",\"MessageAttributes\":{\"x-amzn-trace-id\":{\"Type\":\"String\",\"Value\":\"Root=1-698f814c-7708a2b018bc2cc4726a6288;Parent=f21a582b8b8134b9;Sampled=1\"}}}"
            }]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "698f814c7708a2b018bc2cc4726a6288"
        );
        assert_eq!(hex::encode(&links[0].span_id), "f21a582b8b8134b9");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_from_sns_via_sqs_handles_invalid_body_json() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {},
                "body": "not valid json"
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_from_sns_via_sqs_handles_body_without_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {},
                "body": "{\"Type\":\"Notification\",\"Message\":\"test\",\"MessageAttributes\":{}}"
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: EventBridge ─────────────────────────────

    #[test]
    #[serial]
    fn extract_links_from_eventbridge() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "version": "0",
            "id": "d83d3d45-e768-015d-a133-80d073f5697e",
            "detail-type": "TestMessage",
            "source": "tracing-tests.producer",
            "account": "285732642181",
            "time": "2026-02-04T13:57:53Z",
            "region": "us-west-2",
            "resources": [],
            "detail": {
                "message": "Hello from EventBridge producer!",
                "requestId": "2a45cb5d-0ca6-4a67-aabf-3ff31180a6b2",
                "traceparent": "00-462b7e674cf81fa63fdd74b200000000-cf2870befd9580d7-01"
            }
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "462b7e674cf81fa63fdd74b200000000"
        );
        assert_eq!(hex::encode(&links[0].span_id), "cf2870befd9580d7");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_from_eventbridge_without_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "version": "0",
            "id": "d83d3d45-e768-015d-a133-80d073f5697e",
            "detail-type": "TestMessage",
            "source": "tracing-tests.producer",
            "detail": {
                "message": "Hello from EventBridge producer!"
            }
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_from_eventbridge_with_empty_detail() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "version": "0",
            "id": "d83d3d45",
            "detail-type": "TestMessage",
            "source": "tracing-tests.producer",
            "detail": {}
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: Kinesis ─────────────────────────────────

    #[test]
    #[serial]
    fn extract_kinesis_link_from_amzn_trace_id() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        // Decoded data: {"message":"hello","X-Amzn-Trace-Id":"Root=1-69930da5-56f73ce00e736a0e6081eba8;Parent=462fcf08cfbb8353;Sampled=1","traceparent":"00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"}
        let data_json = serde_json::json!({
            "message": "hello",
            "X-Amzn-Trace-Id": "Root=1-69930da5-56f73ce00e736a0e6081eba8;Parent=462fcf08cfbb8353;Sampled=1",
            "traceparent": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
        });
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(data_json.to_string());
        let payload = serde_json::json!({
            "Records": [{
                "eventSource": "aws:kinesis",
                "kinesis": {
                    "data": data_b64
                }
            }]
        });

        let links = extract_span_links(&payload.to_string());

        assert_eq!(links.len(), 1);
        // Should use X-Amzn-Trace-Id, NOT traceparent
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "69930da556f73ce00e736a0e6081eba8"
        );
        assert_eq!(hex::encode(&links[0].span_id), "462fcf08cfbb8353");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_kinesis_link_falls_back_to_traceparent() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let data_json = serde_json::json!({
            "message": "hello",
            "traceparent": "00-026d2b5d090c15f6423df90800000000-157c058e59db86fb-01"
        });
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(data_json.to_string());
        let payload = serde_json::json!({
            "Records": [{
                "eventSource": "aws:kinesis",
                "kinesis": {
                    "data": data_b64
                }
            }]
        });

        let links = extract_span_links(&payload.to_string());

        assert_eq!(links.len(), 1);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "026d2b5d090c15f6423df90800000000"
        );
        assert_eq!(hex::encode(&links[0].span_id), "157c058e59db86fb");
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_kinesis_links_with_multiple_records() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let data1 = serde_json::json!({
            "traceparent": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
        });
        let data2 = serde_json::json!({
            "traceparent": "00-11112222333344445555666677778888-5555666677778888-01"
        });
        let b64_1 = base64::engine::general_purpose::STANDARD.encode(data1.to_string());
        let b64_2 = base64::engine::general_purpose::STANDARD.encode(data2.to_string());
        let payload = serde_json::json!({
            "Records": [
                {
                    "eventSource": "aws:kinesis",
                    "kinesis": { "data": b64_1 }
                },
                {
                    "eventSource": "aws:kinesis",
                    "kinesis": { "data": b64_2 }
                }
            ]
        });

        let links = extract_span_links(&payload.to_string());

        assert_eq!(links.len(), 2);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "aaaabbbbccccddddeeeeffffaaaabbbb"
        );
        assert_eq!(
            hex::encode(&links[1].trace_id),
            "11112222333344445555666677778888"
        );
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_kinesis_links_handles_invalid_base64() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:kinesis",
                "kinesis": {
                    "data": "!!!not-valid-base64!!!"
                }
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_kinesis_links_handles_non_json_data() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let data_b64 = base64::engine::general_purpose::STANDARD.encode("not json");
        let payload = serde_json::json!({
            "Records": [{
                "eventSource": "aws:kinesis",
                "kinesis": { "data": data_b64 }
            }]
        });

        let links = extract_span_links(&payload.to_string());
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_kinesis_links_handles_missing_trace_context() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let data_json = serde_json::json!({"message": "hello"});
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(data_json.to_string());
        let payload = serde_json::json!({
            "Records": [{
                "eventSource": "aws:kinesis",
                "kinesis": { "data": data_b64 }
            }]
        });

        let links = extract_span_links(&payload.to_string());
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    // ── extract_span_links: mixed / general ─────────────────────────

    #[test]
    #[serial]
    fn extract_links_with_mixed_sqs_and_sns() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [
                {
                    "eventSource": "aws:sqs",
                    "messageAttributes": {
                        "traceparent": {
                            "stringValue": "00-aaaabbbbccccddddeeeeffffaaaabbbb-1111222233334444-01"
                        }
                    }
                },
                {
                    "EventSource": "aws:sns",
                    "Sns": {
                        "MessageAttributes": {
                            "traceparent": {
                                "Type": "String",
                                "Value": "00-11112222333344445555666677778888-5555666677778888-01"
                            }
                        }
                    }
                }
            ]
        }"#;

        let links = extract_span_links(payload);

        assert_eq!(links.len(), 2);
        assert_eq!(
            hex::encode(&links[0].trace_id),
            "aaaabbbbccccddddeeeeffffaaaabbbb"
        );
        assert_eq!(
            hex::encode(&links[1].trace_id),
            "11112222333344445555666677778888"
        );
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_ignores_non_sqs_non_sns_events() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:dynamodb",
                "messageAttributes": {
                    "traceparent": {
                        "stringValue": "00-026d2b5d090c15f6423df90800000000-157c058e59db86fb-01"
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_handles_non_json() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let links = extract_span_links("not json");
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_handles_no_records() {
        std::env::set_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER", "true");
        let links = extract_span_links(r#"{"foo": "bar"}"#);
        assert!(links.is_empty());
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
    }

    #[test]
    #[serial]
    fn extract_links_disabled_when_env_not_set() {
        std::env::remove_var("DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER");
        let payload = r#"{
            "Records": [{
                "eventSource": "aws:sqs",
                "messageAttributes": {
                    "traceparent": {
                        "stringValue": "00-026d2b5d090c15f6423df90800000000-157c058e59db86fb-01"
                    }
                }
            }]
        }"#;

        let links = extract_span_links(payload);
        assert!(links.is_empty());
    }
}

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

/// Extracts a human-readable resource name from an AWS ARN.
///
/// Examples:
/// - `arn:aws:sqs:us-east-1:123:my-queue` -> `my-queue`
/// - `arn:aws:sns:us-east-1:123:my-topic` -> `my-topic`
/// - `arn:aws:kinesis:us-east-1:123:stream/my-stream` -> `my-stream`
/// - `arn:aws:dynamodb:us-east-1:123:table/my-table/stream/2025-...` -> `my-table`
/// - `arn:aws:s3:::my-bucket` -> `my-bucket`
pub fn extract_name_from_arn(arn: &str) -> Option<String> {
    // ARN format: arn:partition:service:region:account:resource
    let parts: Vec<&str> = arn.splitn(7, ':').collect();
    if parts.len() < 6 {
        return None;
    }

    let resource = parts[5..].join(":");
    if resource.is_empty() {
        return None;
    }

    // For DynamoDB: "table/my-table/stream/..." -> "my-table"
    if resource.starts_with("table/") {
        return resource
            .strip_prefix("table/")
            .and_then(|r| r.split('/').next())
            .map(|s| s.to_string());
    }

    // For Kinesis: "stream/my-stream" -> "my-stream"
    if let Some(after_slash) = resource.split('/').last() {
        if resource.contains('/') {
            return Some(after_slash.to_string());
        }
    }

    // For SQS, SNS, S3: resource is the name directly
    Some(resource.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_name_sqs() {
        assert_eq!(
            extract_name_from_arn("arn:aws:sqs:us-east-1:123456789:my-queue"),
            Some("my-queue".to_string())
        );
    }

    #[test]
    fn extract_name_sns() {
        assert_eq!(
            extract_name_from_arn("arn:aws:sns:us-east-1:123456789:my-topic"),
            Some("my-topic".to_string())
        );
    }

    #[test]
    fn extract_name_kinesis() {
        assert_eq!(
            extract_name_from_arn("arn:aws:kinesis:us-east-1:123456789:stream/my-stream"),
            Some("my-stream".to_string())
        );
    }

    #[test]
    fn extract_name_dynamodb() {
        assert_eq!(
            extract_name_from_arn(
                "arn:aws:dynamodb:us-east-1:123456789:table/my-table/stream/2025-01-01T00:00:00.000"
            ),
            Some("my-table".to_string())
        );
    }

    #[test]
    fn extract_name_s3() {
        assert_eq!(
            extract_name_from_arn("arn:aws:s3:::my-bucket"),
            Some("my-bucket".to_string())
        );
    }

    #[test]
    fn extract_name_invalid_arn() {
        assert_eq!(extract_name_from_arn("not-an-arn"), None);
    }

    #[test]
    fn extract_name_empty_resource() {
        assert_eq!(extract_name_from_arn("arn:aws:sqs:us-east-1:123:"), None);
    }
}

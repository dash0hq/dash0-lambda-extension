use crate::otlp::log_mutations::try_read_env_from_file;
use crate::state::invocation_data::{store_metric, StoredMetric};
use crate::state::invocation_entry;
use hyper::header;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::metric::Data;
use opentelemetry_proto::tonic::metrics::v1::{
    AggregationTemporality, Histogram, HistogramDataPoint, Metric, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use prost::Message;

const DURATION_BOUNDS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

const MEMORY_BOUNDS: &[f64] = &[
    0.0, 64.0, 128.0, 256.0, 512.0, 1024.0, 1536.0, 2048.0, 3072.0, 4096.0, 8192.0, 10240.0,
];

fn compute_bucket_counts(value: f64, bounds: &[f64]) -> Vec<u64> {
    let mut counts = vec![0u64; bounds.len() + 1];
    let bucket_index = bounds
        .iter()
        .position(|&b| value <= b)
        .unwrap_or(bounds.len());
    counts[bucket_index] = 1;
    counts
}

fn create_histogram_data_point(
    value: f64,
    attributes: Vec<KeyValue>,
    start_time_unix_nano: u64,
    time_unix_nano: u64,
    explicit_bounds: &[f64],
) -> HistogramDataPoint {
    let bucket_counts = compute_bucket_counts(value, explicit_bounds);
    HistogramDataPoint {
        attributes,
        start_time_unix_nano,
        time_unix_nano,
        count: 1,
        sum: Some(value),
        min: Some(value),
        max: Some(value),
        explicit_bounds: explicit_bounds.to_vec(),
        bucket_counts,
        ..Default::default()
    }
}

fn create_histogram_metric(
    name: &str,
    description: &str,
    unit: &str,
    value: f64,
    attributes: Vec<KeyValue>,
    start_time_unix_nano: u64,
    time_unix_nano: u64,
    explicit_bounds: &[f64],
) -> Metric {
    let data_point = create_histogram_data_point(
        value,
        attributes,
        start_time_unix_nano,
        time_unix_nano,
        explicit_bounds,
    );

    Metric {
        name: name.to_string(),
        description: description.to_string(),
        unit: unit.to_string(),
        data: Some(Data::Histogram(Histogram {
            aggregation_temporality: AggregationTemporality::Delta as i32,
            data_points: vec![data_point],
        })),
        ..Default::default()
    }
}

fn get_metric_attributes() -> Vec<KeyValue> {
    use crate::otlp::attributes::*;
    vec![
        KeyValue {
            key: CLOUD_RESOURCE_ID.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_function_arn()
                        .unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
        KeyValue {
            key: CLOUD_ACCOUNT_ID.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    crate::state::global::get_account_id().unwrap_or_else(|| "unknown".to_string()),
                )),
            }),
        },
    ]
}

pub fn create_metrics(invocation_id: &str) -> Option<StoredMetric> {
    let data = invocation_entry::get_metrics_data(invocation_id)?;

    if data.end_time == 0.0 {
        return None;
    }

    let start_time_unix_nano = ((data.start_time - data.init_duration) * 1_000_000.0) as u64;
    let time_unix_nano = (data.end_time * 1_000_000.0) as u64;

    let mut metrics = Vec::new();

    if data.duration > 0.0 {
        metrics.push(create_histogram_metric(
            "faas.invoke_duration",
            "Duration of the invocation",
            "s",
            data.duration / 1000.0,
            get_metric_attributes(),
            start_time_unix_nano,
            time_unix_nano,
            DURATION_BOUNDS,
        ));
    }

    if data.init_duration > 0.0 {
        metrics.push(create_histogram_metric(
            "faas.init_duration",
            "Duration of the cold start initialization",
            "s",
            data.init_duration / 1000.0,
            get_metric_attributes(),
            start_time_unix_nano,
            time_unix_nano,
            DURATION_BOUNDS,
        ));
    }

    if data.billed_duration > 0.0 {
        metrics.push(create_histogram_metric(
            "dash0.faas.billed_duration",
            "Billed duration of the invocation",
            "s",
            data.billed_duration / 1000.0,
            get_metric_attributes(),
            start_time_unix_nano,
            time_unix_nano,
            DURATION_BOUNDS,
        ));
    }

    if data.memory_usage > 0 {
        metrics.push(create_histogram_metric(
            "faas.mem_usage",
            "Memory used by the invocation",
            "MB",
            data.memory_usage as f64,
            get_metric_attributes(),
            start_time_unix_nano,
            time_unix_nano,
            MEMORY_BOUNDS,
        ));
    }

    if metrics.is_empty() {
        return None;
    }

    let metric_names: Vec<String> = metrics.iter().map(|m| m.name.clone()).collect();

    let scope_metrics = ScopeMetrics {
        scope: Some(InstrumentationScope {
            name: "dash0.lambda-extension".to_string(),
            version: "1.0".to_string(),
            ..Default::default()
        }),
        metrics,
        schema_url: crate::otlp::OTEL_SCHEMA_URL.to_string(),
    };

    let resource = Resource {
        attributes: vec![KeyValue {
            key: crate::otlp::attributes::SERVICE_NAME.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(
                    std::env::var("OTEL_SERVICE_NAME")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .or_else(|| try_read_env_from_file("OTEL_SERVICE_NAME"))
                        .unwrap_or_else(|| "unknown_service".to_string()),
                )),
            }),
        }],
        ..Default::default()
    };

    let export = ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(resource),
            scope_metrics: vec![scope_metrics],
            ..Default::default()
        }],
    };

    let mut headers = hyper::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/x-protobuf"),
    );

    tracing::info!(
        "[{}] Created supplementary metrics for invocation {}: {}",
        crate::log_prefix(),
        invocation_id,
        metric_names.join(", "),
    );

    Some(StoredMetric {
        method: hyper::Method::POST,
        path_and_query: "/v1/metrics".to_string(),
        headers,
        body: export.encode_to_vec(),
    })
}

pub fn create_supplementary_metrics(invocation_id: &str) {
    if let Some(metric) = create_metrics(invocation_id) {
        store_metric(metric);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::invocation_entry;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use opentelemetry_proto::tonic::metrics::v1::metric::Data;
    use prost::Message;
    use serial_test::serial;

    fn reset_store() {
        invocation_entry::remove("inv-metric-1");
    }

    fn setup_invocation(invocation_id: &str, init_duration: f64) {
        invocation_entry::update(invocation_id, |entry| {
            entry.duration = 200.0;
            entry.init_duration = init_duration;
            entry.billed_duration = 300.0;
            entry.memory_usage = 128;
            entry.start_time = 1_000.0;
            entry.end_time = 1_200.0;
        });
    }

    fn find_metric_by_name<'a>(metrics: &'a [Metric], name: &str) -> Option<&'a Metric> {
        metrics.iter().find(|m| m.name == name)
    }

    #[test]
    #[serial]
    fn creates_all_four_histograms_on_cold_start() {
        reset_store();
        let invocation_id = "inv-metric-1";
        setup_invocation(invocation_id, 150.0);

        let stored = create_metrics(invocation_id).expect("should create metrics");
        assert_eq!(stored.path_and_query, "/v1/metrics");

        let decoded =
            ExportMetricsServiceRequest::decode(stored.body.as_slice()).expect("should decode");
        let metrics = &decoded.resource_metrics[0].scope_metrics[0].metrics;
        assert_eq!(metrics.len(), 4);

        // faas.invoke_duration
        let duration_metric = find_metric_by_name(metrics, "faas.invoke_duration").unwrap();
        assert_eq!(duration_metric.unit, "s");
        if let Some(Data::Histogram(h)) = &duration_metric.data {
            assert_eq!(
                h.aggregation_temporality,
                AggregationTemporality::Delta as i32
            );
            let dp = &h.data_points[0];
            assert_eq!(dp.count, 1);
            assert_eq!(dp.sum, Some(0.2));
            assert_eq!(dp.min, Some(0.2));
            assert_eq!(dp.max, Some(0.2));
            assert_eq!(dp.explicit_bounds, DURATION_BOUNDS.to_vec());
            // 0.2s <= 0.25, which is bounds[6], so bucket index 6
            let mut expected_counts = vec![0u64; DURATION_BOUNDS.len() + 1];
            expected_counts[6] = 1;
            assert_eq!(dp.bucket_counts, expected_counts);
            assert_eq!(dp.start_time_unix_nano, 850_000_000); // (1000 - 150) * 1_000_000
            assert_eq!(dp.time_unix_nano, 1_200_000_000); // 1200 * 1_000_000

            // Check attributes — no high-cardinality invocation_id on metrics
            assert!(!dp
                .attributes
                .iter()
                .any(|kv| kv.key == "faas.invocation_id"));
            assert!(dp.attributes.iter().any(|kv| kv.key == "cloud.resource_id"));
            assert!(dp.attributes.iter().any(|kv| kv.key == "cloud.account.id"));
        } else {
            panic!("Expected Histogram data for faas.invoke_duration");
        }

        // faas.init_duration
        let init_metric = find_metric_by_name(metrics, "faas.init_duration").unwrap();
        assert_eq!(init_metric.unit, "s");
        if let Some(Data::Histogram(h)) = &init_metric.data {
            assert_eq!(h.data_points[0].sum, Some(0.15));
        } else {
            panic!("Expected Histogram data for faas.init_duration");
        }

        // dash0.faas.billed_duration
        let billed_metric = find_metric_by_name(metrics, "dash0.faas.billed_duration").unwrap();
        assert_eq!(billed_metric.unit, "s");
        if let Some(Data::Histogram(h)) = &billed_metric.data {
            assert_eq!(h.data_points[0].sum, Some(0.3));
        } else {
            panic!("Expected Histogram data for dash0.faas.billed_duration");
        }

        // faas.mem_usage
        let memory_metric = find_metric_by_name(metrics, "faas.mem_usage").unwrap();
        assert_eq!(memory_metric.unit, "MB");
        if let Some(Data::Histogram(h)) = &memory_metric.data {
            assert_eq!(h.data_points[0].sum, Some(128.0));
        } else {
            panic!("Expected Histogram data for faas.mem_usage");
        }

        // Check resource service.name
        let resource = decoded.resource_metrics[0].resource.as_ref().unwrap();
        let service_name = resource
            .attributes
            .iter()
            .find(|kv| kv.key == "service.name")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| match &v.value {
                Some(Value::StringValue(s)) => Some(s.as_str()),
                _ => None,
            });
        assert!(service_name.is_some());

        // Check instrumentation scope
        let scope = decoded.resource_metrics[0].scope_metrics[0]
            .scope
            .as_ref()
            .unwrap();
        assert_eq!(scope.name, "dash0.lambda-extension");
        assert_eq!(scope.version, "1.0");
    }

    #[test]
    #[serial]
    fn noop_when_invocation_does_not_exist() {
        reset_store();
        let result = create_metrics("inv-nonexistent");
        assert!(result.is_none());
    }

    #[test]
    #[serial]
    fn skips_init_duration_on_warm_start() {
        reset_store();
        let invocation_id = "inv-metric-1";
        setup_invocation(invocation_id, 0.0); // warm start: no init_duration

        let stored = create_metrics(invocation_id).expect("should create metrics");
        let decoded =
            ExportMetricsServiceRequest::decode(stored.body.as_slice()).expect("should decode");
        let metrics = &decoded.resource_metrics[0].scope_metrics[0].metrics;

        assert_eq!(metrics.len(), 3);
        assert!(find_metric_by_name(metrics, "faas.init_duration").is_none());
        assert!(find_metric_by_name(metrics, "faas.invoke_duration").is_some());
        assert!(find_metric_by_name(metrics, "dash0.faas.billed_duration").is_some());
        assert!(find_metric_by_name(metrics, "faas.mem_usage").is_some());

        // Timestamps should use start_time directly (no init_duration subtraction)
        if let Some(Data::Histogram(h)) = &find_metric_by_name(metrics, "faas.invoke_duration")
            .unwrap()
            .data
        {
            assert_eq!(h.data_points[0].start_time_unix_nano, 1_000_000_000); // 1000 * 1_000_000
        }
    }

    #[test]
    #[serial]
    fn skips_all_metrics_when_end_time_is_zero() {
        reset_store();
        let invocation_id = "inv-metric-1";
        invocation_entry::update(invocation_id, |entry| {
            entry.duration = 200.0;
            entry.init_duration = 150.0;
            entry.billed_duration = 300.0;
            entry.memory_usage = 128;
            entry.start_time = 1_000.0;
            entry.end_time = 0.0; // no end_time yet
        });

        let result = create_metrics(invocation_id);
        assert!(result.is_none());
    }
}

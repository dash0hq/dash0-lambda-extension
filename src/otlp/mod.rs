pub const OTEL_SCHEMA_URL: &str = "https://opentelemetry.io/schemas/1.11.0";

pub mod attributes;
pub mod exporter;
pub mod log_mutations;
pub mod logs_receiver;
pub mod masking;
pub mod metrics_creation;
pub mod metrics_receiver;
pub mod receiver;
pub mod resources;
pub mod span_creation;
pub mod span_link_extractor;
pub mod span_mutations;
pub mod trigger_chain;

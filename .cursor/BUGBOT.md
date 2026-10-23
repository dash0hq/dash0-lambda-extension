# Bugbot rules

## Never log telemetry or payload bodies

The receivers (`src/otlp/receiver.rs`, `logs_receiver.rs`, `metrics_receiver.rs`) see OTLP data before the exporter masks it, and the proxy sees Lambda events and return values. Extension logs go to CloudWatch, and at `warn`/`error` also to Dash0, outside any masking.

Flag any `tracing::*!` call that formats a request, response, event, OTLP body or attribute value (e.g. `String::from_utf8_lossy(&body)`, `{:?}` of a decoded request or span). Logging sizes, counts, IDs and error messages is fine. The Telemetry API batches in `src/extension/telemetry_receiver.rs` contain the function's own stdout lines, so they count as payload. `tests/no_payload_in_logs.rs` checks this by identifier name; flag cases it can't see, such as a payload bound to a differently named variable. Also flag new entries in its allowlist that carry function data.

## The export file seam stays debug-only

`DASH0_TEST_EXPORT_DIR` in `send_request` (`src/otlp/exporter.rs`) writes export requests to disk for `tests/masking_e2e.rs`. Flag any change that compiles it outside `#[cfg(debug_assertions)]` or reads it in release builds. Don't flag its use in tests.

## Payload masking runs at export, not on receive

`src/otlp/payload_masking.rs` is called from the export builders in `src/otlp/exporter.rs`. The distros' `forceFlush` waits for `/v1/traces` and `/v1/logs` to return, so masking work in a receiver delays the function's response.

Flag new calls into `payload_masking` from `receiver.rs`, `logs_receiver.rs`, `span_mutations::process_trace_request` or anything else that runs before a receiver returns its `200`. Calls from `exporter.rs` and tests are fine.

# Lambda Extension Metrics

This document describes the metrics emitted by the Dash0 Lambda extension, how they are derived from the AWS Lambda Telemetry API, and how they relate to both the OpenTelemetry FaaS semantic conventions and the upstream `open-telemetry/opentelemetry-lambda` collector.

## Overview

For every Lambda invocation, the extension synthesizes up to four supplementary histogram metrics from the `platform.report` event of the Lambda Telemetry API and forwards them via OTLP/HTTP to the configured exporter. Metrics that customer code emits via OTLP are forwarded unchanged; this document only covers the metrics that the extension itself produces.

Source of truth: [`src/otlp/metrics_creation.rs`](../src/otlp/metrics_creation.rs).

## Emitted metrics

| Metric | Type | Unit | Description | Emission condition |
|---|---|---|---|---|
| `faas.invoke_duration` | Histogram (Delta) | `ms` | Duration of the invocation | `duration > 0` |
| `faas.init_duration` | Histogram (Delta) | `ms` | Duration of the cold-start initialization | `init_duration > 0` (cold starts only) |
| `dash0.faas.billed_duration` | Histogram (Delta) | `ms` | Billed duration of the invocation | `billed_duration > 0` |
| `faas.mem_usage` | Histogram (Delta) | `MB` | Memory used by the invocation | `memory_usage > 0` |

All four metrics are explicit-bucket histograms with **delta** aggregation temporality, `count = 1` per data point, and `sum = min = max` set to the observed value.

The full report record is skipped if `end_time == 0` (the report has not arrived yet).

### Source mapping

Values are taken directly from the Lambda Telemetry API's `platform.report.metrics` object without any unit conversion (see [`src/util/log_processing.rs:133-154`](../src/util/log_processing.rs)).

| Stored field | Telemetry API field | Emitted as | Unit |
|---|---|---|---|
| `data.duration` | `durationMs` | `faas.invoke_duration` | `ms` |
| `data.init_duration` | `initDurationMs` | `faas.init_duration` | `ms` |
| `data.billed_duration` | `billedDurationMs` | `dash0.faas.billed_duration` | `ms` |
| `data.memory_usage` | `maxMemoryUsedMB` | `faas.mem_usage` | `MB` |

## Histogram bucket boundaries

Defined in [`src/otlp/metrics_creation.rs:15-22`](../src/otlp/metrics_creation.rs).

**Duration metrics** (`faas.invoke_duration`, `faas.init_duration`, `dash0.faas.billed_duration`), in milliseconds:

```
0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000
```

**Memory metric** (`faas.mem_usage`), in megabytes:

```
0, 64, 128, 256, 512, 1024, 1536, 2048, 3072, 4096, 8192, 10240
```

## Attributes

### Data-point attributes

Added to every metric data point (see `get_metric_attributes` in [`src/otlp/metrics_creation.rs`](../src/otlp/metrics_creation.rs)):

| Attribute | Type | Description |
|---|---|---|
| `cloud.resource_id` | string | Full ARN of the Lambda function. Falls back to `unknown` if not yet known. |
| `cloud.account.id` | string | AWS account ID. Falls back to `unknown` if not yet known. |

The high-cardinality `faas.invocation_id` is intentionally **not** attached to metric data points.

### Resource attributes

| Attribute | Type | Description |
|---|---|---|
| `service.name` | string | From `OTEL_SERVICE_NAME`; falls back to `unknown_service`. |

### Instrumentation scope

| Field | Value |
|---|---|
| Name | `dash0.lambda-extension` |
| Version | `1.0` |

## OpenTelemetry FaaS semantic conventions

The OpenTelemetry FaaS metrics specification defines the following metrics:

| Metric name (spec) | Instrument | Unit | Description |
|---|---|---|---|
| `faas.invoke_duration` | Histogram | `s` | Function logic execution duration |
| `faas.init_duration` | Histogram | `s` | Function initialization duration (cold start) |
| `faas.coldstarts` | Counter | `{coldstart}` | Number of cold starts |
| `faas.errors` | Counter | `{error}` | Number of invocation errors |
| `faas.invocations` | Counter | `{invocation}` | Number of successful invocations |
| `faas.timeouts` | Counter | `{timeout}` | Number of invocation timeouts |
| `faas.mem_usage` | Histogram | `By` | Distribution of max memory usage per invocation |
| `faas.cpu_usage` | Histogram | `s` | Distribution of CPU usage per invocation |
| `faas.net_io` | Histogram | `By` | Distribution of net I/O usage per invocation |

The spec also recommends the following advisory bucket boundaries for the duration histograms (in seconds):

```
0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1, 2.5, 5, 7.5, 10
```

It does **not** prescribe bucket boundaries for `faas.mem_usage`.

Reference: [semantic-conventions / docs / faas / faas-metrics.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/faas/faas-metrics.md).

## Comparison with the upstream OTel Lambda collector

The [`open-telemetry/opentelemetry-lambda`](https://github.com/open-telemetry/opentelemetry-lambda) project ships a collector with a `telemetryapireceiver` that emits FaaS metrics. The relevant code is [`collector/receiver/telemetryapireceiver/metric_builder.go`](https://github.com/open-telemetry/opentelemetry-lambda/blob/main/collector/receiver/telemetryapireceiver/metric_builder.go), which uses the `go.opentelemetry.io/otel/semconv/v1.25.0` constants — i.e. it emits the spec-conformant names with underscores (`faas.invoke_duration`, `faas.init_duration`, `faas.mem_usage`, etc.).

### Metric coverage

| Metric | Spec name | Dash0 extension | OTel collector |
|---|---|---|---|
| Invoke duration | `faas.invoke_duration` | ✅ | ✅ |
| Init duration | `faas.init_duration` | ✅ | ✅ |
| Memory usage | `faas.mem_usage` | ✅ | ✅ |
| Billed duration | (not in spec) | ✅ as `dash0.faas.billed_duration` | ❌ |
| Cold starts | `faas.coldstarts` | ❌ | ✅ |
| Invocations | `faas.invocations` | ❌ | ✅ |
| Errors | `faas.errors` | ❌ | ✅ |
| Timeouts | `faas.timeouts` | ❌ | ✅ |
| CPU usage | `faas.cpu_usage` | ❌ | ❌ |
| Net I/O | `faas.net_io` | ❌ | ❌ |

### Names and units

| Metric | Dash0 unit | OTel unit | Spec unit | Dash0 name OK? | OTel name OK? | Dash0 unit OK? | OTel unit OK? |
|---|---|---|---|---|---|---|---|
| `faas.invoke_duration` | `ms` | `s` | `s` | ✅ | ✅ | ❌ | ✅ |
| `faas.init_duration` | `ms` | `s` | `s` | ✅ | ✅ | ❌ | ✅ |
| `faas.mem_usage` | `MB` | `By` | `By` | ✅ | ✅ | ❌ | ✅ |
| `dash0.faas.billed_duration` | `ms` | — | n/a (vendor-specific) | n/a | n/a | n/a | n/a |

### Bucket boundaries

| Histogram | Dash0 bounds | OTel collector bounds | Spec advisory |
|---|---|---|---|
| Duration | `[0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000]` ms | `[0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1, 2.5, 5, 7.5, 10]` s | `[0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1, 2.5, 5, 7.5, 10]` s |
| Memory | `[0, 64, 128, 256, 512, 1024, 1536, 2048, 3072, 4096, 8192, 10240]` MB | `[16 MiB, 32 MiB, 64 MiB, 128 MiB, 256 MiB, 512 MiB, 768 MiB, 1 GiB, 2 GiB, 3 GiB, 4 GiB, 6 GiB, 8 GiB]` | (none defined) |

The dash0 duration boundaries are numerically the spec boundaries scaled by 1000 (i.e. they would be exact in seconds), with one extra leading `0`. Because the emitted unit is `ms`, a spec-aware backend that interprets these values as seconds would mis-interpret them by a factor of 1000.

The spec is silent on memory boundaries, so neither implementation is "wrong"; the dash0 set covers Lambda's full 0–10240 MB configurable range, while the OTel collector set extends to lower values (16 MiB) and tops out at 8 GiB.

## Compliance summary

| Aspect | Dash0 extension | OTel collector |
|---|---|---|
| Metric names match spec | ✅ | ✅ |
| Duration unit matches spec | ❌ (emits `ms`, spec requires `s`) | ✅ |
| Memory unit matches spec | ❌ (emits `MB`, spec requires `By`) | ✅ |
| Duration bucket boundaries match advisory | ❌ (right values, wrong unit) | ✅ |
| Counter metrics implemented | ❌ | ✅ |
| Vendor-specific extensions | `dash0.faas.billed_duration` (correctly namespaced) | none |

### Known gaps in the Dash0 extension

1. **Duration unit:** durations are emitted as `ms` but the spec mandates `s`. Either the values should be divided by 1000 (and the unit changed to `s`), or the metrics should be moved out of the `faas.*` namespace.
2. **Memory unit:** `faas.mem_usage` is emitted as `MB` but the spec mandates `By`. Same remedy options as above.
3. **Missing counter metrics:** `faas.invocations`, `faas.coldstarts`, `faas.errors`, and `faas.timeouts` are defined in the spec but not currently produced by the extension.

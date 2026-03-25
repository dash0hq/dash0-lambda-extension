# Dash0 Lambda Extension

An extension for capturing observability data from AWS Lambda invocations and shipping to Dash0.

This extension has four main functionalities:
1. Enable auto-instrumentation for supported runtimes, which currently include Python, Node, Java.
2. Receive traces from auto/manual instrumentations, enrich with data acquired in the extension, and send to Dash0.
3. Detect runtime errors such as timeout or out of memory and create synthetic traces for them
4. Collect all logs and send to Dash0, correlated with the trace id of the invocation.


## Layer ARNs

See the release page for the latest ARNs of the extension layers for each runtime.


## Configuration

### Required

* `AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper` - This environment variable must be set in order to enable tracing. If this environment variable will not be set, only logs will be collected.

* `DASH0_ENDPOINT` - The integration endpoint for you organization in Dash0, i.e. `https://ingress.eu-west-1.aws.dash0.com:4318`.

* `DASH0_TOKEN` - The API token for your Dash0 project.

### Optional

* `DASH0_DISABLE_AUTO_INSTRUMENTATION` - Auto-instrumentation can be turned off by this environment variable, which will result in creating synthetic traces by the extension for all invocations.

* `DASH0_SEND_ON_INVOCATION_END` - The extension has two modes of sending to the backend, either on invocation end or on the next invocation. The default is `true`. Sending on invocation end will increase the billed duration of the lambda, but not the response time. Sending on next invocation will decrease the billed duration since the sending will take place in parallel of the regular execution, but might delay the sending up to 7 minutes in case of last invocation in the container.

* `DASH0_EXTENSION_LOG_LEVEL` - Log level for the extension itself. Valid values: `trace`, `debug`, `info`, `warn`, `error`. Default: `warn`.

* `DASH0_DISTRO_DEBUG` - When set to true, additional logs related to tracing and auto-instrumentation will be emitted. Default: `false`.

* `DASH0_REQUEST_TIMEOUT` - Timeout in milliseconds for HTTP requests to the backend. Default: `2000`.

* `DASH0_MAX_EVENT_PAYLOAD` - Maximum size in KB for event payloads (request/response bodies) captured in traces. Payloads exceeding this limit are truncated. Default: `20`.

* `DASH0_REMOVE_LAMBDA_PARENT_SPAN` - When set to `true` (the default), the extension removes the parent span ID from Lambda server spans for non-sampled invocations. Set to `false` to preserve the original parent span ID.

* `DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER` - When set to `true`, the extension extracts span links from message attributes for SQS, SNS, Kinesis, and adds them to the Lambda server span. Default: `true`.

* `DASH0_CREATE_PAYLOAD_LOG_RECORDS` - When set to `true` (the default), the extension creates log records containing the request and response payloads for the lambda invocation and each client call. Set to `false` to disable. Default: `true`.

* `DASH0_DISABLE_TELEMETRY_LOG_COLLECTION` - When set to `true`, disables collecting logs from the [Lambda Telemetry API](https://docs.aws.amazon.com/lambda/latest/dg/telemetry-api.html). Default: `false`.

* `DASH0_DATASET` - When set, the extension adds a `Dash0-Dataset` header to all OTLP export requests, routing telemetry to the specified dataset in the Dash0 backend.

### Secret Masking

The extension automatically masks sensitive data in traces payloads. By default, any JSON key matching these patterns (case-insensitive) will have its value replaced with `****`:

- `.*pass.*`
- `.*key.*`
- `.*secret.*`
- `.*credential.*`
- `.*passphrase.*`

This is applied to:
- Lambda event payloads
- Lambda response payloads
- Any http request/response payloads captured by the auto-instrumentation

**Custom masking rules:**

* `DASH0_MASK_RULES` - JSON array of regex patterns to customize which keys are masked. When set, this **replaces** the default patterns.

  Example: `DASH0_MASK_RULES='[".*token.*", ".*auth.*", ".*private.*"]'`

* `DASH0_MASK_ENV_VARS` - JSON array of regex patterns specifically for masking environment variables captured in traces. When not set, falls back to using `DASH0_MASK_RULES` (or the defaults).

  Example: `DASH0_MASK_ENV_VARS='[".*PASSWORD.*", ".*API_KEY.*"]'`

**Secret masking in HTTP request and response payloads:**

The following environment variables allow fine-grained control over secret masking in HTTP payloads captured by the auto-instrumentation. Each accepts a JSON array of regex patterns. When not set, they fall back to `DASH0_MASK_RULES` (or the defaults).

* `DASH0_MASK_REQUEST_BODY` - Regex patterns for masking keys in HTTP request bodies.

  Example: `DASH0_MASK_REQUEST_BODY='[".*credit_card.*", ".*ssn.*"]'`

* `DASH0_MASK_REQUEST_HEADERS` - Regex patterns for masking HTTP request header names.

  Example: `DASH0_MASK_REQUEST_HEADERS='[".*authorization.*", ".*cookie.*"]'`

* `DASH0_MASK_RESPONSE_BODY` - Regex patterns for masking keys in HTTP response bodies.

  Example: `DASH0_MASK_RESPONSE_BODY='[".*token.*", ".*session.*"]'`

* `DASH0_MASK_RESPONSE_HEADERS` - Regex patterns for masking HTTP response header names.

  Example: `DASH0_MASK_RESPONSE_HEADERS='[".*set-cookie.*"]'`

* `DASH0_MASK_QUERY_PARAMS` - Regex patterns for masking HTTP query parameter names.

  Example: `DASH0_MASK_QUERY_PARAMS='[".*api_key.*", ".*token.*"]'`


## Enrichment Attributes

The extension enriches telemetry data with additional attributes beyond what the auto-instrumentation provides.

### Span Attributes

The following attributes are added to spans by the extension (if relevant):

| Attribute | Type | Description |
|---|---|---|
| `faas.invocation_id` | string | The AWS request ID of the current invocation. |
| `faas.trigger` | string | The event source that triggered the Lambda (e.g., `aws:sqs`, `aws:dynamodb`, `aws:event_bridge`). Extracted from the event payload. |
| `faas.init_duration` | double | The cold start initialization duration in milliseconds. Only present on cold start invocations. |
| `dash0.faas.record_count` | int | The number of records in a batch event (SQS, DynamoDB Streams, Kinesis, SNS). |
| `dash0.faas.trigger_arn` | string | The ARN of the event source (e.g., SQS queue ARN, DynamoDB stream ARN, SNS topic ARN). |
| `dash0.faas.event_bridge_source` | string | The `source` field from an EventBridge event. |
| `dash0.faas.event_bridge_detail_type` | string | The `detail-type` field from an EventBridge event. |

#### Resource Attributes (Spans)

These attributes are added to the resource of span data:

| Attribute | Type | Description |
|---|---|---|
| `service.name` | string | The service name, from `OTEL_SERVICE_NAME` or defaults to `unknown_service`. |
| `process.environment_variable.<KEY>` | string | Lambda environment variables (with sensitive values masked). Added to the span resource. |

### Log Attributes

The following attributes are added to log records by the extension (if relevant):

| Attribute | Type | Description |
|---|---|---|
| `faas.invocation_id` | string | The AWS request ID, used to correlate logs with the invocation span. |
| `dash0.faas.payload_type` | string | The type of payload log record. Values: `lambda_event`, `lambda_return_value`, `http_request_body`, `http_response_body`. Only present on payload log records. |

#### Resource Attributes (Logs)

These attributes are added to the resource of log data:

| Attribute | Type | Description |
|---|---|---|
| `cloud.platform` | string | Always set to `aws_lambda`. |
| `cloud.resource.id` | string | The full ARN of the Lambda function. |
| `cloud.account.id` | string | The AWS account ID. |
| `service.name` | string | The service name, from `OTEL_SERVICE_NAME` or defaults to `unknown_service`. |

### Metrics

The extension creates the following histogram metrics for each Lambda invocation:

| Metric | Unit | Description |
|---|---|---|
| `faas.duration` | ms | Duration of the invocation. |
| `faas.init_duration` | ms | Duration of the cold start initialization. Only present on cold start invocations. |
| `dash0.faas.billed_duration` | ms | Billed duration of the invocation. |
| `dash0.faas.memory_used` | MB | Memory used by the invocation. |

#### Metric Attributes

The following attributes are added to each metric data point:

| Attribute | Type | Description |
|---|---|---|
| `cloud.resource_id` | string | The full ARN of the Lambda function. |
| `cloud.account.id` | string | The AWS account ID. |

#### Resource Attributes (Metrics)

These attributes are added to the resource of metric data:

| Attribute | Type | Description |
|---|---|---|
| `service.name` | string | The service name, from `OTEL_SERVICE_NAME` or defaults to `unknown_service`. |


## Dockerized Lambdas

For containerized Lambda functions, use the provided Docker images in a multi-stage build. The extension images are available for Node.js, Python, and Java runtimes.

### Node.js

```dockerfile
FROM public.ecr.aws/lambda/nodejs:20

# Copy extension from Dash0 image
COPY --from=dash0/extension-node:latest /opt /opt

# Enable tracing
ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper
ENV DASH0_TOKEN=your-token-here

# Copy your function code
COPY index.js ${LAMBDA_TASK_ROOT}

CMD ["index.handler"]
```

### Python

```dockerfile
FROM public.ecr.aws/lambda/python:3.12

# Copy extension from Dash0 image
COPY --from=dash0/extension-python:latest /opt /opt

# Enable tracing
ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper
ENV DASH0_TOKEN=your-token-here

# Copy your function code
COPY app.py ${LAMBDA_TASK_ROOT}

CMD ["app.handler"]
```

### Java

```dockerfile
FROM public.ecr.aws/lambda/java:21

# Copy extension from Dash0 image
COPY --from=dash0/extension-java:latest /opt /opt

# Enable tracing
ENV AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper
ENV DASH0_TOKEN=your-token-here

# Copy your function code
COPY target/my-function.jar ${LAMBDA_TASK_ROOT}

CMD ["com.example.Handler::handleRequest"]
```


# serverless-dash0

Serverless Framework plugin that automatically adds the [Dash0](https://www.dash0.com) Lambda extension layer and environment variables to your functions.

Compatible with Serverless Framework v3 and v4.

## Installation

```bash
npm install --save-dev @dash0/serverless-dash0
```

## Usage

Add the plugin to your `serverless.yml` and configure the `custom.dash0` section:

> **Where to find your endpoint:** In the Dash0 app, go to **Settings → Endpoints → OTLP via HTTP** and copy the value from the **Endpoint** field.

```yaml
plugins:
  - "@dash0/serverless-dash0"

custom:
  dash0:
    endpoint: https://ingress.eu-west-1.aws.dash0.com:4318
    token: ${env:DASH0_TOKEN}
    layerVersion: latest

functions:
  myFunction:
    handler: handler.hello
    runtime: nodejs20.x
    dash0-traced: true

  untracedFunction:
    handler: handler.other
    runtime: nodejs20.x
```

Only functions with `dash0-traced: true` will have the Dash0 layer and environment variables added. All other functions are left untouched.

### Supported runtimes

| Runtime         | Layer                    |
| --------------- | ------------------------ |
| `nodejs*`       | `dash0-extension-node`   |
| `python*`       | `dash0-extension-python` |
| `java*`         | `dash0-extension-java`   |

Container image functions (`image` instead of `handler`) are not supported. Use the [Dash0 Docker images](https://github.com/dash0hq/dash0-lambda-extension#docker) instead.

## Configuration reference

All options are set under `custom.dash0` in your `serverless.yml`.

### Required

| Option | Type | Description |
| --- | --- | --- |
| `endpoint` | `string` | Dash0 OTLP/HTTP ingestion endpoint. Find it in the Dash0 app under **Settings → Endpoints → OTLP via HTTP → Endpoint**. |
| `layerVersion` | `number` or `"latest"` | Layer version to use. A specific version number, or `"latest"` to use the version bundled with the plugin. |
| `token` | `string` | Dash0 API token. Required unless `tokenSecretArn` is provided. |
| `tokenSecretArn` | `string` | ARN of an AWS Secrets Manager secret containing the Dash0 API token. Required unless `token` is provided. If both are set, both environment variables will be added. |

### Optional

| Option | Type | Default | Env var set | Description |
| --- | --- | --- | --- | --- |
| `tokenSecretKey` | `string` | - | `DASH0_TOKEN_SECRET_KEY` | JSON key to extract the token from the secret (when the secret value is a JSON object). |
| `dataset` | `string` | - | `DASH0_DATASET` | Route telemetry to a specific Dash0 dataset. |
| `extensionLogLevel` | `string` | `warn` | `DASH0_EXTENSION_LOG_LEVEL` | Log level for the extension. One of: `trace`, `debug`, `info`, `warn`, `error`. |
| `disableAutoInstrumentation` | `boolean` | `false` | `DASH0_DISABLE_AUTO_INSTRUMENTATION` | Disable auto-instrumentation. The extension will still create synthetic spans. |
| `sendOnInvocationEnd` | `boolean` | `true` | `DASH0_SEND_ON_INVOCATION_END` | Send telemetry at the end of the current invocation instead of the beginning of the next. |
| `disableTelemetryLogCollection` | `boolean` | `false` | `DASH0_DISABLE_TELEMETRY_LOG_COLLECTION` | Disable Lambda Telemetry API log collection. |
| `disableTelemetryMetrics` | `boolean` | `false` | `DASH0_DISABLE_TELEMETRY_METRICS` | Disable emission of supplementary FaaS metrics. |
| `disableTelemetryTraces` | `boolean` | `false` | `DASH0_DISABLE_TELEMETRY_TRACES` | Disable both auto-instrumentation and synthetic spans (including error-path traces). |
| `createPayloadLogRecords` | `boolean` | `true` | `DASH0_CREATE_PAYLOAD_LOG_RECORDS` | Create log records for request/response payloads. |
| `requestTimeout` | `number` | `2000` | `DASH0_REQUEST_TIMEOUT` | HTTP request timeout in milliseconds. |
| `xrayTracesEnabled` | `boolean` | `false` | `DASH0_XRAY_TRACES_ENABLED` | Preserve X-Ray trace context. |
| `maskRules` | `string` | - | `DASH0_MASK_RULES` | JSON array of regex patterns for masking sensitive data. |
| `maskEnvVars` | `string` | - | `DASH0_MASK_ENV_VARS` | JSON array of regex patterns for masking environment variable values. |

### Behavior

- The plugin runs at the `before:package:initialize` lifecycle hook.
- Existing function layers are preserved; the Dash0 layer is appended.
- Existing function environment variables are never overwritten. If you set `DASH0_EXTENSION_LOG_LEVEL` on a specific function, the plugin will not replace it with the global value.
- Functions with unsupported runtimes or container images are skipped with a warning.

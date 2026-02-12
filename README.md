# Dash0 Lambda Extension

An extension for capturing observability data from lambda invocations and shipping to Dash0.

This extension has four main functionalities:
1. Enable auto-instrumentation for supported runtimes, which currently include Python, Node, Java.
2. Receive traces from auto/manual instrumentations, enrich with data acquired in the extension, and send to Dash0.
3. Detect runtime errors such as timeout or out of memory and create synthetic traces for them
4. Collect all logs and send to Dash0, correlated with the trace id of the invocation.


## Layer ARNs

TBD


## Configuration

### Required

* `AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper` - This environment variable must be set in order to enable tracing. If this environment variable will not be set, only logs will be collected.

* `DASH0_ENDPOINT` - The integration endpoint for you organization in Dash0, i.e. `https://ingress.eu-west-1.aws.dash0.com:4318`.
* 
* `DASH0_TOKEN` - The API token for your Dash0 project.

### Optional

* `DASH0_DISABLE_AUTO_INSTRUMENTATION` - Auto-instrumentation can be turned off by this environment variable, which will result in creating synthetic traces by the extension for all invocations.

* `DASH0_SEND_ON_INVOCATION_END` - The extension has two modes of sending to the backend, either on invocation end or on the next invocation. The default is `true`. Sending on invocation end will increase the billed duration of the lambda, but not the response time. Sending on next invocation will decrease the billed duration since the sending will take place in parallel of the regular execution, but might delay the sending up to 7 minutes in case of last invocation in the container.

* `DASH0_ENDPOINT` - Custom endpoint URL for sending telemetry data. Default: Dash0 ingestion endpoint.

* `DASH0_EXTENSION_LOG_LEVEL` - Log level for the extension itself. Valid values: `trace`, `debug`, `info`, `warn`, `error`. Default: `warn`.

* `DASH0_DISTRO_DEBUG` - When set to true, additional logs related to tracing and auto-instrumentation will be emitted. Default: `false`.

* `DASH0_REQUEST_TIMEOUT` - Timeout in milliseconds for HTTP requests to the backend. Default: `2000`.

* `DASH0_MAX_EVENT_PAYLOAD` - Maximum size in KB for event payloads (request/response bodies) captured in traces. Payloads exceeding this limit are truncated. Default: `20`.

### Secret Masking

The extension automatically masks sensitive data in event and return value payloads. By default, any JSON key matching these patterns (case-insensitive) will have its value replaced with `****`:

- `.*pass.*`
- `.*key.*`
- `.*secret.*`
- `.*credential.*`
- `.*passphrase.*`

**Custom masking rules:**

* `DASH0_MASK_RULES` - JSON array of regex patterns to customize which keys are masked. When set, this **replaces** the default patterns.

  Example: `DASH0_MASK_RULES='[".*token.*", ".*auth.*", ".*private.*"]'`

* `DASH0_MASK_ENV_VARS` - JSON array of regex patterns specifically for masking environment variables captured in traces. When not set, falls back to using `DASH0_MASK_RULES` (or the defaults).

  Example: `DASH0_MASK_ENV_VARS='[".*PASSWORD.*", ".*API_KEY.*"]'`

**Secret masking in http request and response payloads:**


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


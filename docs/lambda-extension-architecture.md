# Dash0 Lambda Extension Architecture Report

## Overview

The **Dash0 Lambda Extension** is an AWS Lambda extension written in Rust that provides observability and monitoring capabilities for Lambda functions across multiple runtimes (Python, Node.js, Java).

The extension acts as an **intelligent proxy** between your Lambda function and AWS, providing:
- **Auto-instrumentation** - Automatically adds tracing without code changes
- **Trace Collection** - Captures distributed traces via OpenTelemetry
- **Log Correlation** - Links logs to traces using trace IDs
- **Runtime Error Detection** - Detects timeouts and OOM errors
- **Metadata Enrichment** - Adds Lambda-specific context to telemetry

---

## Diagram 1: Lambda Lifecycle & Extension Architecture

```mermaid
flowchart TB
    subgraph ENV["AWS LAMBDA EXECUTION ENVIRONMENT"]
        subgraph INIT["INIT PHASE (Not billed for provisioned concurrency)"]
            EXT_INIT["Extension Init<br/>• Load binary<br/>• Register ext<br/>• Start proxy<br/>• Register telemetry"]
            RUNTIME_INIT["Runtime Init<br/>• Load runtime<br/>• Apply wrapper<br/>• Init OTel SDK"]
            FUNC_INIT["Function Init<br/>• Import modules<br/>• Global variables"]

            EXT_INIT --> RUNTIME_INIT --> FUNC_INIT
            FUNC_INIT --> INIT_DUR["init_duration captured<br/>from platform.initReport"]
        end

        subgraph INVOKE["INVOKE PHASE (BILLED DURATION)"]
            subgraph INV_N["Invocation N"]
                direction LR
                PS["platform.start<br/>↓<br/>Capture request<br/>payload"]
                FE["FUNCTION<br/>EXECUTION<br/>↓<br/>Handler executes<br/>+ spans"]
                PRD["platform.runtimeDone<br/>↓<br/>Flush traces<br/>+ logs"]
                PR["platform.report<br/>↓<br/>Capture duration,<br/>memory"]

                PS --> FE --> PRD --> PR
            end

            BILLED["BILLED DURATION (ms)<br/>start_time ────────────▶ end_time"]
            INV_N --> BILLED

            INV_N1["Invocation N+1"]
            INV_N2["Invocation N+2"]

            BILLED -.->|warm container| INV_N1
            INV_N1 -.-> INV_N2
        end

        subgraph SHUTDOWN["SHUTDOWN PHASE"]
            SHUT["Extension receives SHUTDOWN event (spindown)<br/>↓<br/>Final flush of traces/logs"]
        end

        INIT --> INVOKE --> SHUTDOWN
    end

    style INIT fill:#e1f5fe
    style INVOKE fill:#fff3e0
    style SHUTDOWN fill:#fce4ec
    style BILLED fill:#ffcc80
```

### Timing Data Captured by Extension

```mermaid
classDiagram
    class InvocationData {
        +float init_duration
        +float duration
        +float billed_duration
        +float start_time
        +float end_time
        +int memory_usage
    }

    class Source {
        <<telemetry events>>
    }

    Source --> InvocationData : platform.initReport → init_duration
    Source --> InvocationData : platform.report → duration
    Source --> InvocationData : platform.report → billed_duration
    Source --> InvocationData : platform.start → start_time
    Source --> InvocationData : platform.runtimeDone → end_time
    Source --> InvocationData : platform.report → memory_usage
```

---

## Diagram 2: New Lambda Sandbox Startup Sequence

```mermaid
sequenceDiagram
    autonumber
    participant LS as AWS Lambda<br/>Sandbox
    participant EXT as Extension<br/>(lrap binary)
    participant WRAP as Runtime Wrapper<br/>(opt/python/wrapper)
    participant APP as Application<br/>Runtime

    LS->>EXT: Load extension from /opt/extensions/

    Note over EXT: main.rs:69<br/>#[tokio::main]

    EXT->>EXT: Initialize logging (JSON format)
    EXT->>EXT: Latch env variables<br/>(env::latch_runtime_env)
    EXT->>EXT: Start HTTP proxy server<br/>127.0.0.1:9009

    EXT->>LS: POST /2020-01-01/extension/register<br/>{"events":["INVOKE","SHUTDOWN"]}
    LS-->>EXT: Lambda-Extension-Identifier

    EXT->>LS: PUT /2022-07-01/telemetry<br/>destination: 127.0.0.1:9009/v1/telemetry
    LS-->>EXT: Telemetry subscription OK

    LS->>WRAP: Extension registered, start runtime

    Note over WRAP: Set environment:<br/>AWS_LAMBDA_RUNTIME_API=127.0.0.1:9009<br/>LUMIGO_ENDPOINT=http://127.0.0.1:9009/v1/traces<br/>_HANDLER=otel_wrapper.lambda_handler

    WRAP->>APP: Start application runtime

    Note over APP: OTel wrapper initializes<br/>AwsLambdaInstrumentor().instrument()<br/>import ORIG_HANDLER

    APP->>EXT: GET /2018-06-01/runtime/invocation/next
    EXT->>LS: GET /2018-06-01/runtime/invocation/next (proxy)

    Note over LS: BLOCKS until first<br/>invocation arrives

    Note over LS,APP: INIT COMPLETE | PROXY READY | WRAPPER ACTIVE | HANDLER READY
```

---

## Diagram 3: Payload Interception Flow

```mermaid
sequenceDiagram
    autonumber
    participant SRC as Event Source
    participant API as Real Lambda<br/>Runtime API
    participant PROXY as Extension Proxy<br/>(127.0.0.1:9009)
    participant RT as Application<br/>Runtime

    SRC->>API: Invoke

    RT->>PROXY: GET /2018-06-01/runtime/invocation/next

    Note over PROXY: proxy_invocation_next()<br/>route.rs:383

    PROXY->>API: GET /2018-06-01/runtime/invocation/next

    API-->>PROXY: Response:<br/>Header: Lambda-Runtime-Aws-Request-Id: abc-123<br/>Body: {"key": "value"}

    Note over PROXY: INTERCEPT & STORE<br/>1. Extract request ID from header<br/>2. store_current_invocation_id()<br/>3. Truncate if > MAX_EVENT_PAYLOAD (20KB)<br/>4. store_event_payload()

    PROXY-->>RT: Forward unchanged

    Note over RT: FUNCTION EXECUTES

    RT->>PROXY: POST /2018-06-01/runtime/invocation/abc-123/response<br/>Body: {"result": 42}

    Note over PROXY: INTERCEPT RESPONSE<br/>invocation_response_proxy()<br/>route.rs:220<br/><br/>1. Truncate if > MAX_EVENT_PAYLOAD<br/>2. If auto-instr: add_return_payload_to_lambda_server_spans()<br/>3. If manual mode: build_runtime_error_trace()

    PROXY->>API: POST /2018-06-01/runtime/invocation/abc-123/response
    API-->>PROXY: 200 OK
    PROXY-->>RT: 200 OK
```

### In-Memory Storage Structure

```mermaid
erDiagram
    EVENT_PAYLOADS {
        string invocation_id PK
        string payload "JSON request body"
    }

    RETURN_PAYLOADS {
        string invocation_id PK
        string payload "JSON response body"
    }

    CURRENT_INVOCATION_ID {
        string id "Active invocation"
    }

    TRACE_STORE {
        int index PK
        string method
        string path_and_query
        blob headers
        blob body "OTLP protobuf"
        array invocation_ids
    }

    TELEMETRY_LOGS {
        int index PK
        string time
        string type
        json record
        string invocation_id
    }

    INVOCATION_DATA {
        string invocation_id PK
        float init_duration
        float duration
        float billed_duration
        float start_time
        float end_time
        int memory_usage
    }
```

---

## Diagram 4: Data Export Flow

```mermaid
sequenceDiagram
    autonumber
    participant SDK as OTel SDK<br/>(in app)
    participant PROXY as Extension Proxy<br/>(127.0.0.1:9009)
    participant STORE as Store
    participant DASH0 as Dash0 Backend<br/>(HTTPS)

    SDK->>PROXY: POST /v1/traces (OTLP protobuf)

    Note over PROXY: traces() route.rs:266<br/>1. Decode protobuf (or JSON → protobuf)<br/>2. Check for duplicate Java instrumentation<br/>3. process_trace_request()

    Note over PROXY: Add faas.event<br/>Add faas.return_value<br/>Store span IDs

    PROXY->>STORE: store_trace()
    PROXY-->>SDK: 200 OK

    rect rgb(255, 243, 224)
        Note over PROXY: Lambda Telemetry API Events
        PROXY->>PROXY: POST /v1/telemetry (platform events)

        Note over PROXY: Parse telemetry logs:<br/>platform.start → start_time<br/>platform.initReport → init_duration<br/>platform.runtimeDone → end_time<br/>platform.report → duration, billed_duration, memory_usage

        PROXY->>STORE: store_telemetry_logs()
    end

    rect rgb(200, 230, 201)
        Note over PROXY,DASH0: TRIGGER: platform.runtimeDone or platform.report received

        PROXY->>STORE: take_traces()
        STORE-->>PROXY: Vec<StoredTrace>

        Note over PROXY: flush_traces() backend_send.rs:14<br/>1. merge_telemetry_invocation_data()<br/>   - Add faas.init_duration<br/>   - Add faas.billed_duration<br/>   - Add faas.memory_used<br/>   - Adjust start/end times<br/>2. combine_traces()<br/>3. Build OTLP request

        PROXY->>DASH0: POST /v1/traces (OTLP protobuf)<br/>Content-Type: application/x-protobuf<br/>Authorization: Bearer $DASH0_TOKEN
        DASH0-->>PROXY: 200 OK

        PROXY->>DASH0: POST /v1/logs
        DASH0-->>PROXY: 200 OK

        PROXY->>STORE: cleanup_invocation()
    end
```

### Export Configuration

```mermaid
flowchart LR
    subgraph CONFIG["Export Configuration"]
        EP["DASH0_ENDPOINT<br/>(Required)"]
        TK["DASH0_TOKEN<br/>(Required)"]
        TO["DASH0_REQUEST_TIMEOUT<br/>(Default: 2000ms)"]
        RT["DASH0_REQUEST_RETRIES<br/>(Default: 1)"]
        SI["SEND_ON_INVOCATION_END<br/>(Default: true)"]
    end

    CONFIG --> EXPORT["OTLP Export<br/>to Dash0"]

    style CONFIG fill:#e3f2fd
```

---

## Diagram 5: Lambda Runtime Awareness

### 5.1 Environment Variable Integration

```mermaid
flowchart LR
    subgraph LAMBDA_ENV["Lambda Environment"]
        AWS_REGION
        AWS_LAMBDA_FUNCTION_NAME
        AWS_LAMBDA_FUNCTION_VERSION
        AWS_LAMBDA_LOG_STREAM_NAME
        _HANDLER
        AWS_LAMBDA_RUNTIME_API
    end

    subgraph WRAPPER["Wrapper Script Transforms"]
        OTEL_RES["OTEL_RESOURCE_ATTRIBUTES=<br/>cloud.region=$AWS_REGION,<br/>cloud.provider=aws,<br/>faas.name=$AWS_LAMBDA_FUNCTION_NAME,<br/>faas.version=$AWS_LAMBDA_FUNCTION_VERSION,<br/>faas.instance=$AWS_LAMBDA_LOG_STREAM_NAME"]

        ORIG["ORIG_HANDLER=$_HANDLER<br/>_HANDLER=otel_wrapper.lambda_handler"]

        REDIRECT["AWS_LAMBDA_RUNTIME_API=127.0.0.1:9009<br/>(redirect to proxy)"]
    end

    AWS_REGION --> OTEL_RES
    AWS_LAMBDA_FUNCTION_NAME --> OTEL_RES
    AWS_LAMBDA_FUNCTION_VERSION --> OTEL_RES
    AWS_LAMBDA_LOG_STREAM_NAME --> OTEL_RES

    _HANDLER --> ORIG
    AWS_LAMBDA_RUNTIME_API --> REDIRECT

    style LAMBDA_ENV fill:#fff3e0
    style WRAPPER fill:#e8f5e9
```

### 5.2 Extension API Integration

```mermaid
sequenceDiagram
    participant LS as Lambda Sandbox
    participant EAPI as Extension API<br/>(2020-01-01)
    participant EXT as Dash0 Extension<br/>(sandbox.rs)

    EXT->>EAPI: POST /extension/register<br/>{"events":["INVOKE","SHUTDOWN"]}
    EAPI-->>EXT: Lambda-Extension-Identifier
    Note over EXT: Store ID

    loop For each event
        EXT->>EAPI: GET /extension/event/next

        alt INVOKE event
            EAPI-->>EXT: {"eventType": "INVOKE",<br/>"invokedFunctionArn": "arn:aws:lambda:..."}
            Note over EXT: store_function_arn()<br/>Extract account_id
        else SHUTDOWN event
            EAPI-->>EXT: {"eventType": "SHUTDOWN",<br/>"shutdownReason": "spindown"}
            Note over EXT: Final flush
        end
    end
```

### 5.3 Telemetry API Integration

```mermaid
sequenceDiagram
    participant LS as Lambda Sandbox
    participant TAPI as Telemetry API<br/>(2022-07-01)
    participant EXT as Extension Proxy

    EXT->>TAPI: PUT /telemetry (subscribe)
    Note over EXT,TAPI: {"schemaVersion": "2022-07-01",<br/>"destination": {"protocol": "HTTP",<br/>"URI": "http://sandbox.localdomain:9009/v1/telemetry"},<br/>"types": ["platform", "function"]}

    rect rgb(227, 242, 253)
        Note over LS,EXT: Telemetry Events Stream

        LS->>EXT: {"type": "platform.start",<br/>"record": {"requestId": "abc-123"}}
        Note over EXT: update_invocation_data()<br/>→ start_time

        LS->>EXT: {"type": "platform.initReport",<br/>"record": {"initDurationMs": 150.5}}
        Note over EXT: update_invocation_data()<br/>→ init_duration

        LS->>EXT: {"type": "platform.runtimeDone",<br/>"record": {"requestId": "abc-123", "status": "success"}}
        Note over EXT: If status != success:<br/>build_runtime_error_trace()<br/>for timeout/OOM

        LS->>EXT: {"type": "platform.report",<br/>"record": {"metrics": {<br/>"durationMs": 1234.5,<br/>"billedDurationMs": 1300,<br/>"maxMemoryUsedMB": 128}}}
        Note over EXT: update_invocation_data()<br/>→ duration, billed_duration, memory_usage<br/><br/>flush_traces()<br/>cleanup_invocation()
    end
```

### 5.4 Runtime-Specific Instrumentation

```mermaid
flowchart TB
    subgraph PYTHON["Python Runtime"]
        PW["opt/python/wrapper"]
        PO["opt/python/otel_wrapper.py"]

        PW -->|"PYTHONPATH=/opt/python<br/>_HANDLER=otel_wrapper.lambda_handler"| PO
        PO -->|"import lumigo_opentelemetry<br/>AwsLambdaInstrumentor().instrument()"| PS["Scope: opentelemetry.instrumentation.aws_lambda"]
    end

    subgraph NODE["Node.js Runtime"]
        NW["opt/node/wrapper"]
        NI["opt/node/init.mjs"]

        NW -->|"NODE_OPTIONS=--import /opt/node/init.mjs"| NI
        NI -->|"import { NodeSDK } from<br/>'@opentelemetry/sdk-node'"| NS["Scope: @opentelemetry/instrumentation-aws-lambda"]
    end

    subgraph JAVA["Java Runtime"]
        JW["opt/java/wrapper"]
        JA["lumigo-otel-javaagent.jar"]

        JW -->|"JAVA_TOOL_OPTIONS=<br/>-javaagent:/opt/lumigo-otel-javaagent.jar"| JA
        JA --> JS1["Scope: io.opentelemetry.aws-lambda-core-1.0"]
        JA --> JS2["Scope: io.opentelemetry.aws-lambda-events-2.2"]

        JS1 --> DEDUP["drop_duplicate_java_instrumenations()<br/>filters duplicate spans"]
        JS2 --> DEDUP
    end

    subgraph MANUAL["Manual Instrumentation"]
        DIS["DISABLE_AUTO_INSTRUMENTATION=true"]
        DIS --> SYN["Extension creates synthetic traces<br/>for ALL invocations"]
        SYN --> CAP["Captures:<br/>• faas.invocation_id<br/>• cloud.resource_id<br/>• faas.event<br/>• faas.return_value"]
        SYN --> ERR["Detects errors from:<br/>• errorMessage/errorType in response<br/>• statusCode >= 400"]
    end

    style PYTHON fill:#fff9c4
    style NODE fill:#c8e6c9
    style JAVA fill:#ffccbc
    style MANUAL fill:#e1bee7
```

---

## Summary: End-to-End Observability Pipeline

```mermaid
flowchart LR
    subgraph INPUT["Input"]
        ES["Event Source"]
        EP["Event Payload<br/>• JSON body<br/>• Headers<br/>• Context"]
    end

    subgraph LAMBDA["Lambda Runtime"]
        HC["Handler Code<br/>• Business logic<br/>• OTel spans<br/>• Logs"]
    end

    subgraph EXTENSION["Extension (lrap)"]
        EN["Enrichment<br/>• faas.event<br/>• faas.return<br/>• faas.*_duration<br/>• cloud.* attrs"]
    end

    subgraph OUTPUT["Dash0 Backend"]
        ST["Storage &<br/>Visualization<br/>• Traces<br/>• Logs<br/>• Metrics"]
    end

    ES --> EP --> LAMBDA --> HC --> EXTENSION --> EN --> OUTPUT --> ST

    style INPUT fill:#e3f2fd
    style LAMBDA fill:#fff3e0
    style EXTENSION fill:#e8f5e9
    style OUTPUT fill:#fce4ec
```

### Data Enrichment Points

```mermaid
flowchart TB
    subgraph ENRICHMENT["Data Enrichment Pipeline"]
        E1["1. Request Interception<br/>route.rs:383-427<br/>↓<br/>store_event_payload()"]
        E2["2. Trace Reception<br/>route.rs:266-367<br/>↓<br/>process_trace_request()<br/>Add faas.event"]
        E3["3. Response Interception<br/>route.rs:220-264<br/>↓<br/>add_return_payload_to_lambda_server_spans()<br/>Add faas.return_value"]
        E4["4. Telemetry Processing<br/>route.rs:138-217<br/>↓<br/>update_invocation_data()<br/>Extract timing"]
        E5["5. Pre-Export Merge<br/>backend_send.rs:58-92<br/>↓<br/>merge_telemetry_invocation_data()<br/>Add faas.init_duration<br/>Add faas.billed_duration<br/>Add faas.memory_used"]

        E1 --> E2 --> E3 --> E4 --> E5
    end

    style E1 fill:#bbdefb
    style E2 fill:#c8e6c9
    style E3 fill:#fff9c4
    style E4 fill:#ffccbc
    style E5 fill:#e1bee7
```

---

## Code References

| Component | File | Key Functions |
|-----------|------|---------------|
| Entry point | `src/main.rs:69` | `#[tokio::main] async fn main()` |
| HTTP routing | `src/route.rs:28` | `make_route()` |
| Payload interception | `src/route.rs:383` | `proxy_invocation_next()` |
| Response interception | `src/route.rs:220` | `invocation_response_proxy()` |
| Trace processing | `src/route.rs:266` | `traces()` |
| Telemetry processing | `src/route.rs:138` | `telemetry_sink()` |
| Extension registration | `src/sandbox.rs:180` | `extension::register()` |
| Telemetry subscription | `src/sandbox.rs:423` | `extension::register_telemetry()` |
| Trace enrichment | `src/util/span_mutations.rs:316` | `add_event_payload_to_lambda_server_spans()` |
| Export | `src/backend_send.rs:58` | `send_traces()` |
| Storage | `src/store.rs` | All storage functions |

---

*Generated from codebase analysis of dash0-lambda-extension*

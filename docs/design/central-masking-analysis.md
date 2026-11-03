# Central masking in the extension: depth analysis

Status: Phase 1 of the central-masking exploration. Describes `master` at `62ee133`.

## TL;DR

- **The extension's `/v1/traces` handler is on the function's critical path.** The upstream
  `@opentelemetry/instrumentation-aws-lambda` (0.71.0, used by the Node distro) ends the
  invocation span and then runs `Promise.all(flushers).then(callback)`: the handler result is
  handed to the runtime only after `forceFlush()` returned, and `forceFlush()` waits for the HTTP
  POST to `127.0.0.1:9009/v1/traces` to be acked. The Python `AwsLambdaInstrumentor` and the
  Java agent's Lambda instrumentation force-flush the same way. Everything `receiver::traces`
  does before it returns `200` adds to the latency the caller sees.
- **Export happens after the response.** With the default `DASH0_SEND_ON_INVOCATION_END=true` the
  extension waits for `platform.runtimeDone` (which the Telemetry API sends after the runtime
  posted `/response`), then flushes traces, logs and metrics, then calls `/event/next`. Work at
  export time adds to billed duration (`PostRuntimeExtensionsDuration`) but not to the response
  latency. With `DASH0_SEND_ON_INVOCATION_END=false` the flush of invocation N runs when the
  `INVOKE` event of N+1 arrives, concurrently with the N+1 handler (CPU contention, not
  blocking).
- **The pipeline already decodes and re-encodes every OTLP payload at least twice**: once on
  receive (traces: decode → mutate → encode; logs: decode only to validate) and once on export
  (`combine_items` decodes every stored body, the merged request is encoded and gzipped). Node
  sends OTLP/JSON, which costs an extra JSON parse and one redundant protobuf encode.
- **The exporter already holds every span and OTLP log record as decoded structs**
  (`_build_traces_request`, `build_logs_request`, `flush_telemetry_logs`). Masking there adds no
  decode and no encode; it only walks attributes that are already in memory.
- **HTTP bodies never reach the exporter as span attributes.** `extract_http_body_logs` moves
  `http.request.body` / `http.response.body` off every non-Lambda span into `dash0_payload` log
  records (`TelemetryLog`) on receive, and `build_payload_body` parses them as JSON there (on the
  critical path) without masking.
- **The Lambda event is parsed as JSON 4+ times before the handler gets it** (`/next` path, also
  on the critical path): `extract_http_request_data_from_raw_event`, `mask_json_string` (parse +
  serialize), `build_payload_body` (parse + serialize), and later `extract_span_attributes_from_event`
  and `extract_span_links` on the stored masked string.

## Invocation lifecycle

```mermaid
sequenceDiagram
    autonumber
    participant L as Lambda platform<br/>(Runtime + Extensions + Telemetry API)
    participant X as Dash0 extension<br/>(127.0.0.1:9009)
    participant R as Runtime + wrapper + distro
    participant H as Handler
    participant D as Dash0 OTLP endpoint

    X->>L: GET /extension/event/next
    L-->>X: INVOKE (requestId, tracing)
    Note over X: handle_invoke_event; default mode → wait_for_runtime_done()
    R->>X: GET /runtime/invocation/next (proxied)
    X->>L: GET /runtime/invocation/next
    L-->>X: event bytes
    Note over X: CRITICAL PATH (before handler):<br/>parse event for HTTP attrs, mask_json_string,<br/>build lambda_event payload log
    X-->>R: event
    R->>H: invoke
    H-->>R: result
    Note over R: span.end() → forceFlush()
    R->>X: POST /v1/traces (Node: OTLP/JSON, Python/Java: OTLP/protobuf)
    Note over X: CRITICAL PATH (blocks the result):<br/>decode, process_trace_request,<br/>extract_http_body_logs, encode, store
    X-->>R: 200
    R->>X: POST /runtime/invocation/{id}/response
    Note over X: CRITICAL PATH: process_payload(return value)<br/>(mask_json_string parse + serialize)
    X->>L: forward response (caller gets the result now)
    Note over X: payload log + apply_return_value_error_to_stored_traces<br/>(decode/encode, only when statusCode is present)
    X-->>R: 202
    R->>X: GET /runtime/invocation/next (runtime waits here)
    L->>X: POST /telemetry (platform.runtimeDone, function logs)
    Note over X: runtimeDone → flush (POST-RESPONSE, billed):<br/>decode stored bodies, combine, add env vars,<br/>encode, gzip, POST
    X->>D: POST /v1/traces, /v1/logs, /v1/metrics
    X->>L: GET /extension/event/next (sandbox may freeze now)
```

Freeze: once the runtime is in `/next` and every extension has called `/event/next`, the sandbox
can be frozen. Nothing the extension starts after it calls `/event/next` is guaranteed to run until
the next thaw. Any background masking task (option b) must therefore finish before the extension
calls `/event/next`; that is the same point at which the export already happens.

`SHUTDOWN`: only `spindown` flushes (`flush_if_needed`): the extension sleeps 200 ms, then
exports within the 2 s an external extension gets; `deadlineMs` is not read and `timeout` /
`failure` shutdowns do not flush (review-codex.md, objection 3). Export-time masking shares that
budget.

## Data format per hop

| Hop | Format | Code | Copies / transforms |
|---|---|---|---|
| Distro → extension, traces | Node: OTLP/JSON over HTTP; Python, Java: OTLP/protobuf | `receiver.rs` `traces` | `collect().to_bytes()`; `body_bytes.to_vec()` (copy 1) |
| Receive, traces (protobuf) | protobuf → `ExportTraceServiceRequest` → protobuf | `span_mutations::process_trace_request` | decode (allocates every string), mutate, `encode_to_vec` (copy 2) |
| Receive, traces (JSON) | JSON → struct → protobuf → (mutate) → protobuf | `receiver.rs` JSON branch | serde parse, `encode_to_vec` (redundant: overwritten by `process_trace_request`), mutate, encode again |
| Receive, HTTP body attributes | span attribute string → `serde_json::Value` → JSON string inside a `TelemetryLog` | `extract_http_body_logs` → `build_payload_log` → `build_payload_body` | clone of the attribute, JSON parse, JSON serialize; **not masked** |
| Response proxy | response bytes → masked string → log; protobuf → struct → protobuf | `invocation_response_proxy`, `apply_return_value_error_to_stored_traces` | `mask_json_string` parse + serialize **before** the response is forwarded; payload log parse + serialize, decode/encode of the stored trace after it (only when the return value has a `statusCode`) |
| Distro → extension, logs | OTLP/protobuf or JSON (Java agent logs, user SDKs) | `logs_receiver.rs` | `to_vec` copy; protobuf decode **only to validate, result dropped**; JSON → encode |
| Telemetry API → extension | JSON array | `telemetry_receiver.rs` | `from_utf8_lossy`, serde parse into `TelemetryLog` |
| Export, traces | stored protobuf bodies → merged struct → protobuf → gzip | `exporter::_build_traces_request`, `send_request` | decode every stored body, `add_env_vars`, encode, gzip(3) |
| Export, OTLP logs | stored protobuf → merged struct → protobuf → gzip | `exporter::build_logs_request` | decode, encode, gzip |
| Export, telemetry + payload logs | `TelemetryLog` → `LogRecord` → protobuf → gzip | `exporter::flush_telemetry_logs`, `log_mutations::map_logs_to_otlp` | build structs, encode, gzip |

## What is masked today, and where

| Payload | Masked by | Where | Rules |
|---|---|---|---|
| Lambda event (`dash0_payload`, `lambda_event`) | extension | `/next` proxy, `process_payload` → `mask_json_string` | `DASH0_MASK_RULES` / defaults |
| Return value (`lambda_return_value`) | extension | `/response` proxy, `process_payload` | `DASH0_MASK_RULES` / defaults |
| `process.environment_variable.*` | extension | `global::init_env_var_attrs` → `mask_env_vars`, added at export | `DASH0_MASK_ENV_VARS` → `DASH0_MASK_RULES` |
| HTTP headers, bodies, query, messaging, Redis | Node distro (Lumigo-derived hooks); partly Python (PR #117); Java not (PR #118) | inside the instrumentations, before the attribute limit | `DASH0_MASK_*` → `LUMIGO_SECRET_MASKING_REGEX_*` via `map_masking_env` in `opt/shared.sh` |
| `http.request.body` / `http.response.body` moved to payload logs | nobody in the extension | `extract_http_body_logs` | – |
| `http.request.header.<name>`, `url.full`, `url.query`, `http.url`, `http.target`, `db.statement`, `rpc.*.payload`, `gen_ai.*`, OTLP log bodies | nobody (82 of 214 inventory rows are "not masked") | – | – |

The extension reads only `DASH0_MASK_RULES` and `DASH0_MASK_ENV_VARS`. The per-payload variables
(`DASH0_MASK_REQUEST_BODY`, `_RESPONSE_BODY`, `_REQUEST_HEADERS`, `_RESPONSE_HEADERS`,
`_QUERY_PARAMS`) are only translated for the distros by the wrapper.

## Where a span/log processor fits

There is no generic processor chain; the existing mutation passes are:

- `span_mutations::process_trace_request` (receive, per request, critical path)
- `span_mutations::apply_return_value_error_to_stored_traces` (response proxy, decode/encode again)
- `exporter::add_env_vars` (export, on the merged `ResourceSpans`) — **the natural hook**: it already
  runs on decoded structs between `combine_items` and `encode_to_vec`, off the critical path.
- `exporter::build_logs_request` (export, decoded `ResourceLogs`) — same shape for OTLP logs.
- `log_mutations::build_payload_body` (receive) — renders the HTTP body payload logs. A cut-off
  body fails its JSON parse and is stored as a string, so masking the parsed value would miss it;
  the prototype masks the rendered log body at export instead (`exporter::mask_http_payload_logs`).

## Inventory highlights (214 rows, Node 65 / Python 84 / Java 65)

- Status: 61 covered, 58 partial, 82 not masked, 13 not captured.
- Value shapes the extension would see: JSON objects as strings (`http.request.headers`,
  `http.request.body`, `messaging.*.body`), single header values under per-name keys
  (`http.request.header.authorization`), URLs with query strings (`url.full`, `http.url`,
  `http.target`, `url.query`), form-encoded bodies, plain strings (Redis commands, SQS
  `MessageBody`), and arrays (`db.query.parameter.<i>`, `db.statement.parameters`).
- **Truncation:** Node caps every attribute at `OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT` (default
  4096) inside the SDK, so a large JSON body can reach the extension as invalid JSON. Node's
  Lumigo scrubber usually truncates structurally (valid JSON with `✂` markers), but a body it
  could not parse arrives as a JSON string literal wrapping the cut JSON
  (`"{\"password\":\"hun`); Python cuts raw text at 4096, Java at 1024. A strict parser cannot
  mask any of these by key. `util/truncate.rs` only finds terminated strings, so the prototype has
  its own depth-tracking scanner that masks a cut value from its key to the end of the input.
- **Generic attribute-key masking is not safe with the defaults**: `.*token.*` matches
  `gen_ai.usage.input_tokens`, `.*key.*` matches `messaging.kafka.message.key` and
  `aws.dynamodb.*key*`. Masking must target the known payload attributes and the per-name header
  attributes, matching the rule against the header/parameter/JSON key, not the attribute key.

## Cost hotspots (before masking is added)

1. `/next` (critical path, before the handler): 4+ JSON parses of the event, 2 serializations.
2. `/v1/traces` (critical path, after the handler): `to_vec` copy, decode, encode; for Node a JSON
   parse plus two encodes; `extract_http_body_logs` clones each body attribute and parses it as JSON.
3. `/v1/logs` (critical path for distros that flush logs): protobuf decode whose result is dropped.
4. `/response`: `mask_json_string` round trip before the response is forwarded (so on the
   caller's path), then payload log parse/serialize and, when the return value has a
   `statusCode`, decode/encode of every stored trace of the invocation.
5. Export (post-response): decode of every stored body, encode, gzip level 3. Gzip and the
   network round trip dominate this phase.

Phase 3 measures these with criterion benches and a profiler; see [Measurements](#measurements).

## Implications for the design

- Masking on receive (before the `200`) adds directly to the response latency. Masking at export
  adds to post-response billed time only, needs no extra decode, and sees every span of the
  invocation at once.
- HTTP bodies are the exception: they leave the span on receive, into payload logs that are
  exported with the telemetry logs. Masking the rendered log body at export keeps the receive
  path free of masking work.
- The distros keep their attribute length limit, so the extension sees truncated JSON and needs a
  tolerant key scanner, or Node needs a larger limit for body attributes.

## Measurements

Apple M-series laptop, release build. `cargo bench --bench masking` (criterion) and
`cargo test --release --test masking_e2e -- --nocapture` (extension binary against a fake Lambda
API). Lambda gives a 128 MB function about 1/14 of a vCPU, so multiply CPU times by up to ~14
there; the relative numbers hold.

### Masking one payload (default rules)

| Payload | Scanner (new) | Allocs | `mask_json_string` (today's event masking) | Allocs |
|---|---|---|---|---|
| small JSON, 120 B | 198 ns | 1 | 894 ns | 18 |
| JSON, 4 KB | 3.5 µs | 1 | 27.0 µs | 539 |
| JSON cut at 4000 B | 3.4 µs | 1 | 17.9 µs, **returns it unmasked** | 520 |
| JSON 4 KB, no key matches | 3.3 µs | **0** | 27.9 µs | 539 |
| form-encoded | 182 ns | 1 | 47 ns (not parsed, unmasked) | 2 |
| plain text | 4.5 ns | 0 | 47 ns | 2 |
| URL with query | 235 ns | 1 | – | – |

### Pipeline, one invocation of 20 HTTP client spans

| Stage | small bodies | 4 KB bodies |
|---|---|---|
| receive (decode + encode), on the response path | 27.6 µs | 33.0 µs |
| export (decode + encode + gzip) | 52.3 µs | 140 µs |
| export + masking | 70.2 µs | 230 µs |
| masking alone | 21.4 µs (1.1 µs/span) | 94 µs (4.7 µs/span) |
| allocations, export → export + masking | | 544 → 744 (+10/span, one per masked value) |

### End to end (60 invocations, 20 spans each, release binary)

| | masking off p50 / p99 | masking on p50 / p99 |
|---|---|---|
| `/v1/traces` ack (the handler's flush waits on it) | 697 / 838 µs | 688 / 835 µs |
| `/response` ack | 279 / 455 µs | 285 / 384 µs |
| `platform.runtimeDone` → next `/event/next` (billed, post-response) | 250 / 350 µs | 310 / 462 µs |

### Profile (samply, `export+mask/json_4k`)

| Self/inclusive | |
|---|---|
| 35% | gzip (`miniz_oxide` deflate + crc32) |
| 46% | masking, of which ~20% the byte scanner and ~21% regex matching before the per-body key cache (the cache took masking from 118 µs to 94 µs) |
| 12% | protobuf decode + encode |
| 7% | malloc/free, memmove |

### Is there an unnecessary decode/re-encode, and does masking add one?

- **Existing, not caused by masking:** every trace payload is decoded and re-encoded on receive
  (`process_trace_request`) and again at export; the Node JSON path encodes twice on receive
  (the first `encode_to_vec` in `receiver.rs` is overwritten); `/v1/logs` decodes only to
  validate; the Lambda event is parsed 4+ times on `/next`. Removing the receive-side re-encode
  (store the decoded request instead of bytes) would save ~30 µs per flush on the response path
  and is independent of masking.
- **Masking adds none.** It runs on the structs the exporter already decoded, between
  `combine_items` and `encode_to_vec`. JSON-in-string attributes are scanned as bytes, not
  parsed into a tree, and rewritten only when a key matched.

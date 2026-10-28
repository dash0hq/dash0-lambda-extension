# Central masking in the extension: options and decision

Status: decided for the prototype. Read with
[`central-masking-analysis.md`](central-masking-analysis.md) (lifecycle, formats, measurements)
and the two reviews, [`review-codex.md`](review-codex.md) (performance, freeze, shutdown) and
[`review-claude.md`](review-claude.md) (correctness against the inventory).

## Decision

**Option (c): mask at export time, on the structs the exporter has already decoded**, behind
`DASH0_MASK_IN_EXTENSION=true` (default off). With the flag on, the distros stop masking (see
[Handover from the distros](#handover-from-the-distros)).

- Spans and span events: `exporter::_build_traces_request`, after `add_env_vars`, before
  `encode_to_vec`. Every trace export goes through it, including the error path in
  `telemetry_receiver` and synthetic/supplementary spans.
- OTLP log records (attributes and body): `exporter::build_logs_request`.
- HTTP body payload logs (`dash0_payload`, `http_request_body` / `http_response_body`):
  `exporter::flush_telemetry_logs`, on the rendered log body's `message`.
- Masking code: `src/otlp/payload_masking.rs`. Rules are resolved once; JSON in strings is masked
  by a byte scanner that also handles cut-off JSON, splices replacements in place (key order,
  numbers and `✂` markers survive) and does not allocate when nothing matches.

Measured cost: none on the response path (`/v1/traces` ack p50 697 µs off vs 688 µs on); +60 µs
p50 post-response per invocation of 20 spans in the e2e harness; 94 µs of CPU for 20 spans with
4 KB bodies in criterion, against 140 µs for the decode + encode + gzip the export already does.
Masking adds no protobuf decode or encode.

## Goal and priorities

1. Do not block the function: no masking work between the handler's return and the caller
   receiving the response.
2. Measure it, and add no decode/encode round trip.
3. One place: the instrumentations stay unaware.

## Options compared

| | (a) on receive | (b) ack, then background | **(c) at export** | (d) in distros (today) |
|---|---|---|---|---|
| Where | `/v1/traces` handler, before `200` | task spawned by the receiver | exporter, on merged decoded structs | each instrumentation |
| Response latency | **adds**: the distro's `forceFlush` waits for the ack (Node, Python and Java verified) | none directly; competes for the same vCPU | none (default mode) | adds, inside the handler |
| Extra (de)serialization | none for protobuf | a second decode unless the decoded request is handed over (large refactor) | none | n/a |
| Freeze / `SHUTDOWN` | nothing pending | task must be joined before `/event/next` on every flush path | runs inside the existing flush | n/a |
| Correctness | sees what the distro sent | adds ordering hazard with `apply_return_value_error_to_stored_traces` | same input as (a); raw data stays in extension memory until export | three regex dialects, three truncation behaviours, Java none |
| Verdict | rejected (priority 1) | rejected (complexity, no gain over c) | **chosen** | baseline; keep during migration |

With `DASH0_SEND_ON_INVOCATION_END=false`, the export of invocation N runs at the start of N+1,
concurrently with its handler. Masking then competes for CPU with the next handler; it never
blocks it.

## What gets masked, and with which rules

Each variable resolves once at start-up: the per-payload variable if valid, else
`DASH0_MASK_RULES` if valid, else the defaults (same as `map_masking_env` in `opt/shared.sh`).
`all` replaces the whole value (for URLs: the query string only). `[]` masks nothing for that
payload type and is distinct from unset. Invalid patterns are dropped with a warning each; a list
whose patterns are all invalid falls back to the next variable instead of masking nothing.

| Attribute | Rules | How |
|---|---|---|
| `http.request.body`, `rpc.request.payload`, `rpc.grpc.request.payload`, `db.request.body`, `messaging.publish.body` (not RabbitMQ) | `DASH0_MASK_REQUEST_BODY` | JSON by key (also cut off, also wrapped in a JSON string literal), form fields by name, plain text only by `all` |
| `db.statement` / `db.query.text` when `db.system` is `redis` | `DASH0_MASK_REQUEST_BODY` | plain text: `all` only. SQL is never touched |
| `http.response.body`, `messaging.consume.body`, `rpc.response.payload`, `rpc.grpc.response.payload`, `db.response.body` | `DASH0_MASK_RESPONSE_BODY` | as request body |
| `http.request.headers` (JSON), `http.request.header.<name>`, `rpc.grpc.request.metadata.<key>` | `DASH0_MASK_REQUEST_HEADERS` | JSON by key; per-name attributes by the name suffix (string or string array) |
| `http.response.headers`, `http.response.header.<name>`, `rpc.grpc.response.metadata.<key>` | `DASH0_MASK_RESPONSE_HEADERS` | as request headers |
| `http.request.query` (JSON), `url.query`, `url.full`, `http.url`, `http.target` | `DASH0_MASK_QUERY_PARAMS` | query parameters by percent-decoded name; scheme, host, path, fragment kept |
| `messaging.produce.body`, `messaging.publish.body` with `messaging.system=rabbitmq` | `DASH0_MASK_QUERY_PARAMS` | as body (keeps Node's footnote-2 behaviour) |
| Java `messaging.message.payload` / `.headers` / `.body` | producer span: query params (body: request body); consumer span: response body | direction comes from the span kind; unknown kind is not masked |
| `dash0_payload` HTTP body logs | request / response body | the log's `message` only, wrapper keys untouched |
| OTLP log record body | `DASH0_MASK_RULES` | map bodies and JSON text by key; prose untouched; `all` is not applied to log bodies |
| Lambda event, return value, env vars | unchanged | unchanged |

Attribute keys are never matched against the rules: the defaults would hit
`gen_ai.usage.input_tokens` (`.*token.*`) and `messaging.kafka.message.key` (`.*key.*`).

Not covered (explicit, tested as expected gaps where the inventory lists them):
`exception.message`/`.stacktrace`, `gen_ai.*`/`llm.*`/`traceloop.*` prompts,
`db.query.parameter.<i>`/`db.statement.parameters` (no keys), `messaging.header.<name>`,
`messaging.url`/`db.connection_string` credentials, `http.client_ip`, function stdout from the
Telemetry API, log attributes with user-chosen keys.

## Reviewer objections and how they were resolved

| # | Objection | Resolution |
|---|---|---|
| Codex 1, Claude 4 | Receivers print raw bodies (`/v1/logs` and `/v1/metrics` at info, `/v1/traces` at trace) to CloudWatch. | **Fixed**: they log sizes only. `tests/masking_e2e.rs` runs at info level and asserts no secret reaches stdout. Bugbot rule added. |
| Codex 2, Claude 2 | `util/truncate.rs` is not a tolerant masker; truncated bodies parsed into `Value::String` are missed; Node wraps unparsable bodies in a JSON string literal. | **Fixed**: own depth-tracking scanner; masks cut values to the end; unwraps one string-literal layer; decodes escaped keys; a test cuts a body at every byte and asserts no secret prefix leaks. |
| Claude 3, 9 | Payload-log masking in `build_payload_body` is on the critical path and misses truncated bodies. | **Fixed**: masked at export on the rendered log body. No receive-path masking remains. |
| Claude 1 | Mapping misses Java/gRPC/stable-semconv keys and RabbitMQ. | **Fixed** except the out-of-scope list above; table driven by `src/otlp/testdata/masking_inventory.json` (from the 214-row inventory). |
| Claude 5 | `serde_json` round trip reorders keys, loses big numbers and `✂` markers. | **Fixed** by splicing; the scanner never re-serializes. |
| Claude 6 | `all` / `[]` / invalid regex semantics. | **Fixed** as described above. `none` is not a value and was dropped. |
| Claude 7, 13 | Strings and form bodies start being masked by key; query strings in `url.full` are new. | **Accepted** behaviour change behind the flag; documented in `docs/masking.md`. |
| Claude 8 | `db.statement` is also SQL. | **Fixed**: Redis only. |
| Claude 10, Codex 6, 7 | Cost is not "far smaller than gzip"; regex count per key. | **Measured**: masking is 46% of export+mask CPU for 4 KB bodies, gzip 35%. One joined regex plus a per-body key cache cut masking from 118 µs to 94 µs for 20 spans. The "dominates" claim was removed. |
| Claude 11 | Header arrays, kvlists, bytes; extension-captured API Gateway headers get masked too. | **Fixed** (all shapes walked); masking the extension's own captured headers is intended. |
| Claude 12 | Double masking while distros still mask. | Idempotent: `"****"` under a matching key stays. Outputs can differ where the extension masks more. |
| Codex 3 | Shutdown: no deadline, `timeout`/`failure` skip the flush, retries can exceed 2 s. | **Pre-existing, out of scope.** Masking never sends unmasked data to meet a deadline. Follow-up. |
| Codex 4 | `runtimeDone` notifier races with late telemetry and the response-proxy tail. | **Pre-existing, out of scope.** Masking sits in the final builders, so whatever is exported is masked. Follow-up. |
| Codex 5 | With `SEND_ON_INVOCATION_END=false` CPU contends with the next handler. | Documented above; measured cost is ~5 µs per span with 4 KB bodies on a laptop core, ~70 µs at 128 MB. |
| Codex 8 | Drain-before-send loses batches on failure; eviction; no byte cap. | **Pre-existing, out of scope.** No raw requeue path was added. |

## Handover from the distros

With `DASH0_MASK_IN_EXTENSION` truthy (`1`, `true`, `yes`, `y`, any case, as in
`payload_masking::rules()`), `map_masking_env` in `opt/shared.sh` exports
`LUMIGO_SECRET_MASKING_REGEX` and every `LUMIGO_SECRET_MASKING_REGEX_*` as `[]` and skips the
`DASH0_MASK_*` mapping. Both Lumigo libraries read `[]` as "no rules"; an unset variable would
fall back to their built-in defaults. User-set `LUMIGO_*` values are overridden. No distro code
reads the flag.

| Runtime | Masked before | With the flag on |
|---|---|---|
| Node.js | every Dash0 hook through `@lumigo/node-core` `payloadStringify` / `scrubHttpPayload` | nothing masked; `payloadStringify` still truncates structurally (`✂`, valid JSON) |
| Python | every Dash0 hook through `lumigo_core` `lumigo_dumps*` (`libs/json_utils.py`) | nothing masked; `lumigo_dumps` still truncates (`...[too long]`) |
| Java | nothing (`SecretScrubber` and `ProcessEnvironmentScrubber` had no caller) | unchanged; the dead scrubbers are removed |

Nothing is lost: every Node and Python attribute the inventory marks as distro-masked is
`expect: masked` in `masking_inventory.json`. Its `gap` rows for those runtimes (SQS bodies
holding JSON strings, non-JSON AWS SDK bodies, psycopg row tuples, the FastAPI
`http.request.query_string`) are not masked by the distros either. Lumigo does not parse JSON
inside string values or masks plain strings by key.

Tested by `opt/test/shared.test.sh` (mapping), `opt/node/test/masking-env.test.ts` and
`opt/python/distro/src/test/unit/libs/test_masking_env.py` (distro output unmasked, still cut),
`tests/masking_e2e.rs` `extension_masks_what_the_distros_leave_unmasked` (the distros' real
captures, `tests/testdata/distro_captured_{node,python}.json`, go through the binary and come out
masked), and the `masking-in-extension-*` integration functions.

Node cost of the scrubber, 20 JSON bodies of 3.5 KB cut at 2048 B, laptop: default rules
1.4 ms, `[]` 0.75 ms, `JSON.parse` + `JSON.stringify` + slice 0.21 ms, slicing a string that is
already serialized ~0. Keeping Lumigo with `[]` halves the in-handler cost with no distro change.
Replacing it with plain serialization and a cut saves another ~0.5 ms per 20 bodies. That is part
of the follow-up below, because it changes every hook and the cut shape (the extension's scanner
handles mid-token cuts).

Debug span dumps (`DASH0_DEBUG_SPANDUMP`) bypass the extension; with the flag on they are
unmasked.

### Dead once the flag defaults on

The follow-up PR that flips the default can delete:

- `opt/shared.sh`: `map_masking_env`, `is_valid_mask_value`, `valid_mask_value_of`,
  `DASH0_DEFAULT_MASK_RULES` (keep exporting the `LUMIGO_*` variables as `[]` while the Lumigo
  libraries stay), and their tests in `opt/test/shared.test.sh`.
- Node: the `ScrubContext` arguments and masking semantics of `payloadStringify` /
  `scrubHttpPayload` in `tools/payloads.ts`, `https/http.ts`, `express`, `fastify`, `undici`,
  `aws-sdk/hooks.ts`, `amqplib`, `kafkajs`, `redis`, `ioredis`. Replace them with serialization
  plus a cut at the attribute limit. Drop `opt/node/test/masking-env.test.ts`, apart from the
  capture fixture.
- Python: the `context` parameter and `dump_with_context` in `libs/json_utils.py` and its callers
  (botocore parsers, django, fastapi, flask, httpx, pymongo, requests). Replace them with
  `dump`. The `lumigo_core` dependency stays for `parse_triggers`.
- Docs: the per-runtime sections of `docs/masking.md` that describe distro masking, and the
  "Payloads that are not masked" lists.

## Follow-ups (not in this prototype)

1. Store the decoded request on receive instead of re-encoding (saves ~30 µs per flush on the
   response path; independent of masking).
2. Shutdown deadline discipline and flushing on `timeout`/`failure` (Codex 3).
3. Keyed `runtimeDone` completion and ordering with the response-proxy tail (Codex 4).
4. Measure on real Lambda at 128 / 512 / 1769 MB, arm64 and x86_64.

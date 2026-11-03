# Review: central masking in the extension (correctness challenge)

Reviewer: Claude. Scope: `central-masking-analysis.md` and the DRAFT `central-masking-plan.md`,
checked against `master` at `62ee133`, the 214-row distro inventory (`inv-all.json`) and
`docs/masking.md`. Read-only; no code was changed or run beyond reading.

## Verdict

**Agree with (c) as the place for span-attribute and OTLP-log masking. Disagree with the plan as
written.** The mapping table would leak data once the distros stop masking, the "truncated JSON"
model is wrong for Node, the payload-log part of (c) runs on the critical path the plan promises
to keep clear, and several statements ("`all` and `none` keep their documented meaning", "far
smaller than gzip", "the tolerant scanner already finds keys") don't hold up against the code.

None of this argues for (a), (b) or (d). Most objections are about *what* gets masked and *how*.
Those apply to every option. The two that are specific to (c), #3 and #9, have contained fixes.
**Precondition:** distros keep masking until the extension passes a per-attribute conformance
test built from the inventory (objection #12).

## Claims checked

| Claim | Verdict | Evidence |
|---|---|---|
| `/v1/traces` is on the critical path | Holds | `receiver.rs:14-130` does all work before the `200` at `:130`. |
| Export runs after `runtimeDone` (default) or at the next `INVOKE` | Holds | `events.rs:109-133`, `events.rs:85-107`. |
| Exporter holds decoded structs; masking adds no decode/encode | Holds for protobuf | `exporter.rs:187-206` (`combine_items` → `add_env_vars` → `encode_to_vec`), `exporter.rs:146-163`. JSON-string attributes still need a parse, plus a serialize or splice of their own. |
| Every export path goes through those two builders | Holds | `flush_traces`/`send_traces` (`exporter.rs:21,165`), also the error path `telemetry_receiver.rs:84-106`; synthetic and supplementary spans (`span_creation.rs:81` copies `handler_attributes`) are exported through `send_traces` too. |
| HTTP bodies leave the span on receive, unmasked | Holds, with an exception | `span_mutations.rs:307-360`, called only for non-Lambda scopes (`:789-795`). The body **stays on the span** when no invocation is current (`:308-311`) and is dropped if it is not a `StringValue` (`:357-358`). |
| `build_payload_body` parses the body as JSON | Partly | `log_mutations.rs:179-182,224-258`. A truncated body fails the parse and becomes `Value::String`, so there are no keys to check (see #3). |
| "The extension already has a tolerant key scanner" | Overstated | `truncate.rs:21-53` finds *terminated* string literals and detects keys. It drops an unterminated string (`:28-37`), so a value cut by the limit is never found. It also has no nesting tracking, so it can't mask an object or array value whole, which `docs/masking.md:36` requires. |
| Node sends truncated, invalid JSON | Wrong for most Node payloads (see #2) | `opt/node/distro/src/tools/payloads.ts:22-41` → Lumigo `payloadStringify` / `scrubWithApproximateMaxPayloadSize` (`@lumigo/node-core/lib/common/httpUtils.js:37-63`) truncates structurally and emits valid JSON with `✂` markers. |
| "`all` and `none` keep their documented meaning" | Wrong | `none` is not documented anywhere (`docs/masking.md`, `README.md`, `opt/shared.sh:49-54` accept only `all` or an array). The extension rejects `all` in `DASH0_MASK_RULES` and falls back to the defaults (`masking.rs:43-56`, documented at `docs/masking.md:305-306`). |
| Masking cost "far smaller" than gzip and network | Unsupported (see #10) | No measurement yet. A rough estimate puts it in the same order as gzip at 128 MB. |

## Objections, ranked by severity

### Critical: data leaks once the distros stop masking

**1. The mapping table misses attributes and maps some to the wrong variable.** The distros pick the
rule set from the library, not from the attribute key. The extension only sees the key, so the
table needs `messaging.system` / `db.system` / span-kind conditions. Failure scenarios:

- **`messaging.publish.body` is two payload classes.** SQS uses `DASH0_MASK_REQUEST_BODY`, but
  `amqplib` (Node) and `pika` (Python) use `DASH0_MASK_QUERY_PARAMS` (`docs/masking.md:53,56`,
  footnote ²; inventory rows `amqplib`, `pika`). With `DASH0_MASK_QUERY_PARAMS=all` set as
  `docs/masking.md:724` instructs, the plan masks RabbitMQ publishes with the request-body rules,
  which are the defaults. The message, a string, goes out in clear text.
- **Java attributes are missing.** `messaging.message.payload` (Kafka/RabbitMQ, the rule set
  depends on PRODUCER vs. consumer span kind), `messaging.message.body` (SQS receive),
  `messaging.message.headers` (Kafka, `name=[bytes]` list) and `messaging.header.<name>`. All are
  in the inventory. The plan names only the Node and Python keys.
- **gRPC keys differ.** Python and Java use `rpc.grpc.request.payload` / `rpc.grpc.response.payload`
  and `rpc.grpc.*.metadata.<key>`. The plan lists only Node's `rpc.request.payload`.
- **Stable semconv renames.** `db.query.text` replaces `db.statement` under
  `OTEL_SEMCONV_STABILITY_OPT_IN=database`; Java Lettuce already emits it (inventory). It isn't in
  the table, so Redis commands under stable semconv are not masked.
- **Also missing:** `http.request.path` (FastAPI), `messaging.url` (amqplib, credentials in the
  AMQP URL), `db.connection_string` (Node redis), and `http.client_ip` / `client.address`. The
  last two are PII the plan should list as out of scope on purpose.

**2. The model of the input is wrong for Node, and truncation is handled differently in each
runtime.** The extension receives at least four shapes for the same logical body:

| Source | Shape at the extension |
|---|---|
| Node, body that fits | Valid JSON. Over-budget values are cut to `value✂`, or the whole value becomes `"✂"` (Lumigo structural truncation). |
| Node, JSON body truncated *before* scrubbing (multi-chunk; response longer than the limit; inventory "VERIFIED GAP") | `JSON.parse` fails (`payloads.ts:30-37`), and `payloadStringify` of a string emits a **JSON string literal** that wraps the truncated JSON: `"{\"password\":\"hun`. |
| Python | Raw truncated text, cut at 4096 before masking (`instrumentations/instrumentation_utils.py:7-18`). |
| Java | Cut at 1024 (`otel.span.attribute.value.length.limit`); its masker closes the JSON (inventory). |

A scanner run on the outer value of the Node string literal finds one string and no keys,
because the quotes inside are escaped. The plan's "tolerant scan if truncated" therefore misses
exactly the Node case the inventory verified as leaking. Also, if "distros stop masking" means
removing the Lumigo calls in Node, the structural truncation goes with them. Then the SDK's
4096-character cut (`utils.ts:96-102`) produces raw invalid JSON for every large body, a worse
input than today's.

**3. The payload-log part of (c) is on the critical path and does not see truncated bodies.**
`extract_http_body_logs` → `build_payload_log` → `build_payload_body` runs inside the
`/v1/traces` handler (`span_mutations.rs:790-795`, before `receiver.rs:130`). That contradicts
priority 1. Masking "the already-parsed `serde_json::Value`" fails for any truncated body:
`payload_message` returns `Value::String` (`log_mutations.rs:179-182`), so the tolerant scanner
would have to run there, which is the expensive case on the critical path. And
`build_payload_body` is shared with `lambda_event` / `lambda_return_value`, which
`process_payload` already masked with `DASH0_MASK_RULES` (`runtime_proxy.rs:203,326`). Masking
there has to branch on `payload_type`, or the event gets the request-body rules applied as
well.

**4. Logging of raw bodies in the extension.** Today these lines leak data the distros didn't mask.
After the switch they would leak everything:

- `logs_receiver.rs:20-24` logs the full `/v1/logs` body at **`info`**.
- `metrics_receiver.rs:20-24` logs the full `/v1/metrics` body at `info`.
- `receiver.rs:24-28` logs the full `/v1/traces` body at `trace`.

Any customer who sets `DASH0_EXTENSION_LOG_LEVEL=info` to debug gets every unmasked body in
CloudWatch Logs. At `warn`/`error`, the extension subscribes to its own `extension` log stream
(`register.rs:193-199`), so any future `warn!` that formats a payload is shipped to Dash0
unmasked as a telemetry log. (`telemetry_receiver.rs:25-31` logs function logs, which are
already in CloudWatch, so it adds no new exposure.) The plan says "dropped or masked". It should
say **removed, replaced by sizes and counts**, and a Bugbot rule should keep it that way.

### High: semantic regressions against `docs/masking.md`

**5. Re-serialising through `serde_json::Value` changes the payload.** `serde_json` is built
without `preserve_order` or `arbitrary_precision` (`Cargo.toml:23`), so `mask_json_string`
(`masking.rs:108-118`) has these effects:

- **Key order:** keys are sorted alphabetically. Every `http.request.body` example in
  `docs/masking.md` (for example `:188-189`, original order `email, password, apiToken, orderId`)
  would become false. Today only the Lambda event is documented as sorted (`:606`).
- **Large numbers:** integers above `u64::MAX` become `f64`, and Snowflake/Twitter-style 64-bit
  IDs inside larger numeric types lose precision.
- **Markers:** `✂` markers and whitespace are rewritten.

"Re-serialised only when a key matched" doesn't help, because with the defaults almost every
body has a matching key. **Splice the masked value's byte range in place instead of
round-tripping.** That also handles truncated input with the same code.

**6. `all` / `[]` / invalid values change meaning.**

- **`all` in `DASH0_MASK_RULES`:** today the extension falls back to the defaults
  (`docs/masking.md:305-306`) and Python breaks (`:290`). If the extension now honours `all`, the
  Lambda event and return value turn into `"****"` too. That change is reasonable, but it is a
  behaviour change: the docs must say so.
- **`all` output:** it must produce the JSON string literal `"****"` that the docs show
  (`:300,367`), not bare `****`. For `url.full`, define whether `all` replaces the query or the
  whole URL (recommendation: the query only).
- **`[]`:** turns masking off in the extension for `DASH0_MASK_RULES` (`docs/masking.md:638`).
  For `DASH0_MASK_ENV_VARS`, though, an empty list falls back to the global rules
  (`masking.rs:127-132`), because "empty" and "unset" are the same `Vec`. Copying that code for
  the five per-payload variables gives `DASH0_MASK_REQUEST_BODY='[]'` a meaning the docs don't
  state. Pick one meaning and represent "unset" as `Option`.
- **Invalid regexes are dropped silently** (`masking.rs:29-36`). Lumigo compiles JS regexes and
  Python compiles `re`. The Rust `regex` crate supports neither look-around nor backreferences, so
  `["(?<!x-)token"]` works in Node today and becomes **an empty rule set in the extension: masking
  off, no warning**. Moving masking into Rust narrows the accepted syntax. The extension must warn
  per dropped pattern and must fall back, not turn masking off, when *every* pattern failed.

**7. Strings start being parsed.** `docs/masking.md:38` and `:709-719` promise that strings are not
parsed. If the extension JSON-parses every payload attribute, this happens:

- A Java or Python Kafka message, a raw `{"email":…}` string, starts being masked by key.
- The same message from Node, a double-encoded string literal, does not.
- If the extension also unwraps string literals to fix #2, Node Kafka messages and SQS
  `MessageBody` start being masked by key too.

Either result can be fine, but the plan must choose one, apply it to all three runtimes, and
rewrite `:38`, `:640-651` and `:709-735`. Form-encoded bodies, which the plan masks "by name", are
the same kind of change: today they are `all`-only in all three runtimes (inventory, Python
`mask_text`; Node `payloads.ts:43-47`).

**8. `db.statement` is SQL as well as Redis.** Without a `db.system == redis` check,
`DASH0_MASK_REQUEST_BODY=all` (which the docs recommend for HTTP, `:768`) blanks every SQL and
MongoDB statement. By-key parsing would start masking MongoDB commands, which `docs/masking.md:781`
says are "not masked, not even `all`". The same check is needed for `db.response.body`.

### Medium: specific to (c)

**9. Moving body extraction to export needs the trace's invocation id, not the current one.** The
fix for #3 is to keep `http.*.body` on the stored span and extract it in `_build_traces_request`.
But `extract_http_body_logs` reads `get_current_invocation_id()` (`span_mutations.rs:308`). With
`DASH0_SEND_ON_INVOCATION_END=false`, export of N runs at `INVOKE` N+1 (`events.rs:90-105`), so the
logs would be filed under N+1. `flush_logs(Some(current))` then excludes that id
(`exporter.rs:31-38`, `invocation_entry.rs:301-311`) and the logs are delayed by an invocation,
or lost if the sandbox is shut down before the next flush. Use `StoredTrace.invocation_ids[0]`.
The ordering (traces before logs) already holds on all three paths (`events.rs:104-105,130-131`,
`telemetry_receiver.rs:104-106`).

**10. CPU cost is not "far smaller", and in one mode it is on the handler's time.** Rough estimate
for a 4 KB JSON body with about 100 keys:

- Parse: about 15 µs.
- 8 unanchored regexes × 100 keys: about 40 µs.
- Serialize or splice: about 10 µs.

That is about 70 µs of CPU per attribute. For 50 spans × 3 payload attributes, that is about
10 ms of CPU. At 128 MB (about 1/14 vCPU) that becomes **about 150 ms of wall time**, the same
order as gzip level 3 on the same 600 KB. In the default mode this is billed post-response time.
With `DASH0_SEND_ON_INVOCATION_END=false`, it competes with the N+1 handler for the same vCPU, so
priority 1 is not met in that mode. Mitigations are cheap:

- Use one `RegexSet` per rule set.
- Memoise each key's result in a per-flush `HashMap<&str, bool>`, since header and JSON keys
  repeat heavily.
- Skip attributes that contain no `"`, `=`, `?` or `:` byte.

Measure with the Phase 3 benches before claiming a number.

**11. Header and array shapes.** `http.request.header.<name>` is a `string[]` in semconv, and Java
emits arrays (inventory: "whole value list → `["****"]`"). The extension's own API Gateway capture
emits a folded string (`http_attributes.rs:306-345`). The plan says "single value", but the
walker must handle `ArrayValue`, and also `KvlistValue` and `BytesValue` (Python dumps bytes as
int arrays; inventory boto3 row). Masking `http.request.header.<name>` at export also masks the
headers the extension captured from the raw event (`runtime_proxy.rs:319-324`). That closes the
gap `docs/masking.md:608-609` warns about, which is good, but it is a documented behaviour
change. The same applies to `url.query` from `DASH0_CAPTURE_API_GATEWAY_QUERY_STRING`. Response
headers are masked twice: once with `DASH0_MASK_RULES` on the return value (`runtime_proxy.rs:203`)
and again with `DASH0_MASK_RESPONSE_HEADERS` at export. The union contradicts "a per-payload
variable replaces the rules" (`docs/masking.md:42`) by masking more, not less.

### Low

**12. Double masking while distros still mask** is safe only if both sides resolve the same rule
set, and the result is idempotent: `"****"` under a matching key stays `"****"`. It breaks in
three cases. #1 picks a different variable than the distro, which masks more. Python's anchored
`re.match` is a subset of Rust's search, so the extension masks more. Node's `✂` markers must
survive the extension untouched. None of these leak data. But they change outputs, so the
`docs/masking.md` examples need to be re-generated from the combined pipeline, not from Node
alone (`CLAUDE.md` "Generate the examples by running the real code").

**13. URL query masking details.** For `url.full`, `http.url` and `http.target`:

- Match the *decoded* parameter name: `pass%77ord`, `+`, repeated names, names without a value.
- Keep the fragment, path and userinfo handling the same as upstream (`REDACTED`).
- `****` is a valid query character.

With the defaults, `.*key.*` masks `keyword=`, `monkey=`, and S3 presigned `X-Amz-Credential` /
`X-Amz-Signature` / `X-Amz-Security-Token`. That is wanted, but it is new: today Node masks only
the parsed `http.request.query`.

**14. Not covered at all**, so the plan should say so instead of implying "one place":

- `exception.message` / `exception.stacktrace` span events, which often embed request bodies.
- `gen_ai.*`, `llm.*` and `traceloop.*` prompts.
- `db.postgresql.values`, `db.statement.parameters` and `db.query.parameter.<i>` (arrays with no
  keys).
- Function stdout via the Telemetry API.
- OTLP log record **bodies** (`KvlistValue` bodies could be masked by key with `DASH0_MASK_RULES`;
  string bodies can't).
- Log attributes from MDC or `extra=`, whose keys are user-chosen and not semconv.

## Alternatives

- **(a) Mask on receive.** Same correctness work as (c), with the cost on the response latency:
  about 70 µs × payload attributes × 14 at 128 MB, so 3 bodies come to about 3 ms. Acceptable only
  for the payload logs, and #9 makes even that unnecessary.
- **(b) Ack first, mask in the background.** Adds the ordering hazard with
  `apply_return_value_error_to_stored_traces` (`span_mutations.rs:619-682` decodes and re-encodes
  stored bodies on `/response`) and join handles on every flush path, including the
  `telemetry_receiver` error path. No correctness gain over (c).
- **(d) Distros only.** Three regex dialects and three truncation behaviours (#2, #6), and Java
  stays unmasked until PR #118. Keep it only as defence in depth during the migration.

## Recommended changes to the plan

1. **Replace the mapping table** with one keyed on (attribute key, guard), generated from
   `inv-all.json`. Guards: `messaging.system`, `db.system` and span kind. Add every Java, Python
   and stable-semconv key from #1. Add an explicit "never mask" list
   (`messaging.kafka.message.key`, `gen_ai.usage.*`, `aws.dynamodb.*`) and an explicit "out of
   scope" list (#14).
2. **Define one input normaliser** for every payload attribute:
   1. If the value is a JSON string literal, unwrap it once.
   2. If what remains starts with `{` or `[`, run a structural, depth-tracking scanner that
      tolerates truncation and splices masked byte ranges in place (#5). A value cut by the limit
      is masked from its key to the end of the input. `✂` is kept.
   3. Otherwise, if it looks like `k=v&…`, mask the form fields by name.
   4. Otherwise apply `all` only.

   Write down what this changes in `docs/masking.md` (#7) and apply it the same way in all three
   runtimes.
3. **Move HTTP body extraction to export.** Keep `http.*.body` on the stored span, then in
   `_build_traces_request` mask it, build the payload log, and file it under
   `StoredTrace.invocation_ids[0]` (#3, #9). This takes all masking work off `/v1/traces`.
4. **Remove body logging** at `receiver.rs:24`, `logs_receiver.rs:20` and `metrics_receiver.rs:20`,
   and log sizes instead (#4). Add a scoped Bugbot rule: flag any `tracing::*!` that formats a
   request, response or OTLP body.
5. **Define rule resolution in a table** (#6):
   - `Option<Rules>`, so `[]` and unset are distinct.
   - `all` per variable, including `DASH0_MASK_RULES`, with the event/return-value change
     documented.
   - Per-pattern warnings for regexes the Rust crate rejects.
   - Fall back to the next rule set when a list compiles to zero rules.

   `none` is not a value; drop it from the plan.
6. **Walk every value shape** (`StringValue`, `ArrayValue`, `KvlistValue`, `BytesValue`) on span
   attributes, span event attributes, link attributes and log record attributes and bodies, not
   only span attributes (#11, #14).
7. **Performance:** use a `RegexSet`, a per-flush key cache and a byte pre-filter, and add a
   criterion bench at 50 spans × 3 payloads (#10). Note that
   `DASH0_SEND_ON_INVOCATION_END=false` moves this CPU onto the next handler.
8. **Gate the distro switch-off** on a conformance test: replay one recorded attribute value per
   inventory row through the extension and assert the docs' output. Keep Node's Lumigo structural
   truncation (pass empty rules; don't remove the call) so #2 doesn't get worse. Switch off one
   runtime at a time.
9. **Update `docs/masking.md` in the same change** (repo rule): the ¹ footnote ("doesn't mask them
   again"), `:38`, `:305-306`, `:608-609`, `:640-651` and the runtime markers.

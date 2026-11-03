# Extension masking: test matrix

Every section and example of [`docs/masking.md`](../masking.md) traces to a test that runs it through the extension with `DASH0_MASK_IN_EXTENSION=true`. Where the docs show a before/after example, the test uses the docs' payload as input and asserts the docs' output exactly. The docs show the payloads pretty-printed; the tests use them as compact JSON, which is how they are stored as attributes.

There are four test locations:

- **docs**: `src/otlp/payload_masking_docs_tests.rs`, unit tests of `mask_attributes`. Tests are named `docs_<section>`.
- **pipeline**: `src/otlp/exporter.rs`, `mod pipeline`. This runs receive, then `process_trace_request`, then export in-process, so it includes the HTTP bodies that move to `dash0_payload` logs.
- **e2e**: `tests/masking_e2e.rs`, `every_mask_variable_shapes_the_exported_payloads`. It runs the real binary against a fake Lambda API and fake distro, and reads back what the extension exports. Each `DASH0_MASK_*` variable is one scenario.
- **integration**: `integration-tests/tests/src/test-masking-in-extension.test.ts`. One deployed function per runtime (`masking-in-extension-{node,python,java}`) with `DASH0_MASK_IN_EXTENSION=true` sends a secret only where its distro doesn't mask it: the URL query in all three, the quoted form body in Node.js, and the `requests` headers and JSON body in Python. The test reads the spans and logs back from Dash0, checks each one is masked, and checks that the secret appears nowhere. It runs only in CI, on a non-draft PR. Its first run found two gaps the in-process tests had missed:
  - The Python distro writes `requests` headers as a Python dict repr, not JSON. The scanner now accepts single-quoted reprs.
  - The Node distro writes form bodies as a quoted string. These are now masked by field name.

  A third finding is left as a known gap: a `Location` response header repeats the request URL, query included. The test skips that one value.
- **inventory**: `src/otlp/payload_masking_inventory_tests.rs`, covering every attribute the three distros write (see [Inventory](#inventory)).

## Docs sections

| `docs/masking.md` section | Example | Tests |
|---|---|---|
| Defaults: nothing configured | headers, request body, SQS publish; query, response headers and body, consume ("same pattern") | docs `docs_defaults`; pipeline `defaults_mask_every_payload_and_nothing_else`; e2e scenario `defaults` |
| `DASH0_MASK_RULES`: only your own patterns | request body, request headers | docs `docs_mask_rules_own_patterns`; pipeline `custom_mask_rules_replace_the_defaults_everywhere`; e2e `DASH0_MASK_RULES=email` |
| `DASH0_MASK_RULES`: your patterns plus the defaults | request body, SQS consume | docs `docs_mask_rules_patterns_plus_defaults` |
| `DASH0_MASK_RULES`: `all` | request body → `"****"`, and every other payload | docs `docs_mask_rules_all`; pipeline `all_hides_whole_payloads`; e2e `DASH0_MASK_RULES=all` |
| `DASH0_MASK_RULES`: `all` note (event, return value, env vars use the defaults) | | e2e `DASH0_MASK_RULES=all` (event, return value and env vars asserted as defaults) |
| `DASH0_MASK_REQUEST_BODY`: patterns | request body, SQS publish, response body unchanged | docs `docs_request_body_patterns`; e2e `DASH0_MASK_REQUEST_BODY=[".*email.*"]` |
| `DASH0_MASK_REQUEST_BODY`: `all` | request body, SQS publish (queue URL too) | docs `docs_request_body_all`; e2e `DASH0_MASK_REQUEST_BODY=all` |
| `DASH0_MASK_RESPONSE_BODY`: patterns | response body, SQS consume | docs `docs_response_body_patterns`; e2e `DASH0_MASK_RESPONSE_BODY=[".*email.*"]` |
| `DASH0_MASK_RESPONSE_BODY`: `all` | response body, consume | docs `docs_response_body_all`; e2e `DASH0_MASK_RESPONSE_BODY=all` |
| `DASH0_MASK_REQUEST_HEADERS`: patterns | request headers (authorization no longer masked) | docs `docs_request_headers_patterns`; e2e `DASH0_MASK_REQUEST_HEADERS=[".*email.*"]` |
| `DASH0_MASK_REQUEST_HEADERS`: `all` | request headers → `"****"` | docs `docs_request_headers_all`; e2e `DASH0_MASK_REQUEST_HEADERS=all` |
| `DASH0_MASK_RESPONSE_HEADERS` | response headers, patterns and `all` | docs `docs_response_headers`; e2e `DASH0_MASK_RESPONSE_HEADERS=…` (4 scenarios) |
| `DASH0_MASK_QUERY_PARAMS` | `http.request.query` | docs `docs_query_params`; e2e `DASH0_MASK_QUERY_PARAMS=…` (4 scenarios, also `url.full`) |
| Combining variables | request body, response body, request headers | docs `docs_combining_variables`; pipeline `each_variable_controls_only_its_payload` |
| `DASH0_MASK_ENV_VARS` | the docs' four variables with the docs' list | e2e `DASH0_MASK_ENV_VARS` (exact values); every e2e scenario checks env vars as on master |
| Lambda event and return value | API Gateway event, `body` string untouched, keys sorted | docs `docs_lambda_event` (exact); e2e: every scenario, with the flag on and unset, asserts the master behaviour |
| Invalid values | `.*email.*`, `[".*email.*",]`, `[.*email.*]`, `[null]` → defaults; `[]` turns masking off | docs `docs_invalid_values_fall_back`; pipeline `invalid_values_fall_back_to_mask_rules`; e2e `DASH0_MASK_RULES invalid` and `<variable>={not json` (5 scenarios) |
| Payloads that are strings: SQS messages | `MessageBody` / `^Body$` patterns | docs `docs_sqs_messages` |
| Payloads that are strings: Kafka and RabbitMQ | patterns (see disagreement 1), `QUERY_PARAMS=all`, `RESPONSE_BODY=all` | docs `docs_kafka_message_with_patterns`, `docs_kafka_and_rabbitmq_all` |
| Payloads that are strings: Redis commands | patterns leave the command; `REQUEST_BODY=all` masks it; object replies by key | docs `docs_redis_commands` |
| Payloads that are not masked | still unmasked with `all` everywhere: `db.postgresql.values`, MySQL `db.statement`, `db.statement.parameters`, FastAPI `http.request.query_string` | docs `docs_payloads_that_stay_unmasked` (asserted unchanged as known gaps) |
| Payloads that are not masked | masked by the extension: gRPC payloads, MongoDB commands, Python HTTP bodies and headers | docs `docs_payloads_the_extension_masks_in_every_runtime` |
| Masking in the extension (experimental) | every example in the section | docs `docs_extension_section_examples` generates them (`-- --nocapture` prints the Markdown); pipeline `disabled_exports_payloads_as_received`; OTLP log records, including `all` leaving log bodies alone: pipeline `otlp_logs_are_masked_on_export` |
| `DASH0_MASK_IN_EXTENSION` unset | nothing new masked | pipeline `disabled_exports_payloads_as_received`; e2e `flag unset` |

### E2E scenarios

There are 26 scenarios, each run against the real binary for one invocation. For each one the test asserts these exported payloads: `url.full`, `http.request.query`, `http.request.headers`, `http.request.header.x-api-key`, `http.response.headers`, the request and response body `dash0_payload` logs, the `lambda_event` and `lambda_return_value` logs, and the `process.environment_variable.*` resource attributes. The extension's stdout and stderr must not contain the secret or the email address.

- `flag unset`
- `defaults`
- `DASH0_MASK_RULES=[".*email.*"]`
- `DASH0_MASK_RULES=all`
- `DASH0_MASK_RULES` invalid
- `DASH0_MASK_ENV_VARS`
- For each of `DASH0_MASK_QUERY_PARAMS`, `_REQUEST_HEADERS`, `_RESPONSE_HEADERS`, `_REQUEST_BODY` and `_RESPONSE_BODY`: `[".*email.*"]`, `all`, `[]` and `{not json`. The variable's own payloads must follow the variable; every other payload must keep the defaults.

The exporter sends only over HTTPS, so the test reads the payloads through a seam compiled into debug builds only: `DASH0_TEST_EXPORT_DIR` writes each export request to a file instead of sending it. The scenario test is `#[cfg(debug_assertions)]`, so `cargo test --release` (latency numbers) skips it.

## Inventory

`src/otlp/testdata/distro_inventory.json` lists the 214 attribute rows the Node.js, Python and Java distros write. `src/otlp/testdata/masking_inventory.json` has 293 entries. Compound attributes such as `a / b`, `<name>` and `<i>` are expanded into concrete keys, and each entry carries a realistic value that contains the secret.

| expect | Entries | Node.js | Python | Java | Asserted |
|---|---:|---:|---:|---:|---|
| `masked` | 161 | 50 | 56 | 55 | secret gone with the defaults; a custom rule replaces the defaults; `all` replaces the value (for a URL, the query); `[]` leaves it |
| `gap` | 72 | 14 | 31 | 27 | unchanged, with a reason (known gap) |
| `not-sensitive` | 50 | 27 | 21 | 2 | unchanged, with a reason |
| `elsewhere` | 10 | 3 | 3 | 4 | unchanged at export: the Lambda event and return value, env vars, the Java spandump file |

By inventory row: 126 rows are only masked, 43 only gap and 20 only not-sensitive. Another 15 rows are split across two expects; for example, SQS payload keys are masked while the `MessageBody` string is a gap. 8 rows are `elsewhere`, and 2 `(none)` rows are skipped on purpose.

Completeness tests fail when an inventory row has no fixture entry, or when an entry points at no row.

One gap was promoted to masked: MongoDB `db.statement` / `db.query.text` (with `db.system` or `db.system.name` = `mongodb`) is now masked as a request body.

These gaps were left as they are because masking them safely is not cheap:

| Gap | Why |
|---|---|
| FastAPI `http.request.query_string` | It is a JSON-quoted raw query. |
| `messaging.url`, `db.connection_string` | They need a userinfo shape. |
| `messaging.header.<name>` | The key has no direction. |
| Elasticsearch `db.query.text` | `.*key.*` would match DSL words such as `keyword`. |
| Java Kafka byte-array headers | They need a new parser. |

## Docs and extension disagreements

1. **Kafka and RabbitMQ messages that hold JSON.** `docs/masking.md` says their keys are not checked. The extension unwraps a JSON string literal that holds JSON and masks it by key. This is now listed in the experimental section, and `docs_kafka_message_with_patterns` asserts it. The per-runtime text is unchanged, and it is correct for the distros.
2. **Python query strings.** The "Payloads that are not masked" table lists Flask and FastAPI query strings under `http.request.query`. FastAPI writes `http.request.query_string`, and Flask captures none. The per-runtime row was left for a docs owner to fix. The experimental section states the real key.
3. **"Payloads that are not masked".** With the flag on, gRPC messages, MongoDB commands and the Python HTTP payloads are masked because they use the payload attributes. The experimental section and a note under that heading now say so.
4. **Invalid patterns.** The docs say the extension drops an invalid pattern without a warning. That is true for the master path (event, return value, env vars). The payload rules behind the flag log one warning per invalid pattern.
5. **Python `pika` publishes.** These are classified as RabbitMQ and so follow `DASH0_MASK_QUERY_PARAMS`, as Node's `amqplib` does. The docs list Python RabbitMQ as not masked, which is true for the distro.

## CloudWatch impact: before vs after

CloudWatch receives the function's stdout and stderr, and the extension's own stdout and stderr.

| What reaches CloudWatch | Before (distro masking only) | After (`DASH0_MASK_IN_EXTENSION=true`) |
|---|---|---|
| Function stdout/stderr (`console.log`, `print`, loggers) | Not masked | Not masked. The extension masks only what it exports, and never CloudWatch. It forwards the same lines to Dash0 as log records, also unmasked. |
| Extension log at the default `warn` | No payload content | No payload content |
| Extension log at `info`, `debug` and `trace` | The telemetry receiver printed every Telemetry API batch at `info`, including the function's own log lines. That copied function output (and any secret in it) into CloudWatch a second time. The trace receiver printed raw bodies. | Sizes only. Fixed in this PR. `extension_log_has_no_payload_content_at_any_level` runs the binary at `trace`, `debug` and `info`, with the flag on and off, and a function log line that contains the secret. |
| `DASH0_DEBUG_SPANDUMP` (distro setting) | Spans as the distro masked them, written to the named file | Unchanged: the dump is written before the extension sees the spans. If distro masking is later removed in favour of the extension, the dump contains raw values. Documented in `docs/masking.md`. |

Guards:

- `tests/no_payload_in_logs.rs` fails when a `tracing` macro in `src/` formats a body, payload, record or attribute value. Logging `.len()` is allowed. The test has an explicit allowlist for Extensions API control messages, which carry no function data.
- The e2e tests assert that the extension's output contains no secret at any level.

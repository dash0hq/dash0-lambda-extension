# Python distro masking benchmark

Measured 2026-10-23 on Apple M4 Pro, 14 CPU cores, 48 GiB RAM, macOS 26.6.2 (25G83), arm64. Existing venv: CPython 3.13.11. Other workers were active on this laptop; no CPU pinning, governor control, or isolation. These are local latency samples, not Lambda measurements.

Source commit: `abcc83f474a8f95760fe805a6148af4bdbeceffe`. Packages: `lumigo-core==0.0.16`, `opentelemetry-sdk==1.43.0`, `opentelemetry-instrumentation-requests==0.64b0`.

## Reproduce

The bench scripts are not in the repo. They are on branch `archive/masking-bench-kits`, under `docs/design/distro-bench/python-bench/`: run `git show origin/archive/masking-bench-kits:docs/design/distro-bench/python-bench/<file>` to get one, or check out that branch. Run `bench.py`, then `report.py`, with the distro venv's Python from a checkout of PR #117.

Exact shared fixture: `benchmarks/masking/fixtures/masking-payloads.json`. SHA-256: `38c627841dc5ec172570938b9501b4c9a8956de7cc121b7e501f3cef07382df7`. Input lengths in UTF-8 bytes: form_encoded=51, json_4k=4111, non_matching_json_4k=4118, plain_text=32, small_json=116, truncated_json_4k=4000.

## Method and scope

Each timed operation has 1,500 warm-up calls, then 20,000 individually timed calls (spans: 10,000; 20-span invocations: 2,000). Timing uses `perf_counter_ns`; GC is disabled only during timing. p50 is the median and p99 is nearest-rank of individual calls, not a percentile of batch averages. Calls include the timer/function-call overhead (no subtraction). The large JSON is parsed, recursively copied/masked, serialized with the real `lumigo_core.lumigo_utils.aws_dump`, and subject to the real 4,096-character budget. No payloads are invented or padded.

Default mode sets the fixture default pattern array in `DASH0_MASK_RULES` and `LUMIGO_SECRET_MASKING_REGEX`, equivalent to the wrapper mapping; custom mode uses exactly `[".*email.*"]` through the same variables. Custom rules **replace** the defaults. Context overrides and legacy rules are cleared. The measured environment had both OpenTelemetry attribute length variables unset (the 4,096-character default); the reproduction script pins them to 4,096. Shipped mode re-reads context and global environment variables on every call, with `_parse_rule` already warm. Cached mode changes only the in-memory benchmark module's `get_rule` to return a precomputed rule for each context; it is an experiment, not a source change or proposed implementation.

Allocation columns are median **peak additional live traced bytes** and net live traced bytes retained after the call, over 101 separately measured calls, with `tracemalloc` and collection before each call. They include Python objects/temporaries visible to tracemalloc, exclude untraced native allocations, and are **not cumulative bytes allocated or allocation-event counts**. Retained values include the returned value, CPython object freelists, and small measurement effects; they are not solely output size. Peak is the useful working-memory estimate. Timing runs do not enable tracemalloc.

## Per payload: mask_text (HTTP body text path)

| Rules | Payload | Shipped p50 ns | Shipped p99 ns | Peak B | Retained B | Cached p50 ns | Cached p99 ns | Cached peak B |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| default | form_encoded | 833 | 1,042 | 1,104 | 216 | 166 | 250 | 120 |
| default | json_4k | 162,167 | 219,250 | 56,651 | 19,622 | 161,208 | 221,125 | 56,651 |
| default | non_matching_json_4k | 162,875 | 223,541 | 56,940 | 19,622 | 161,375 | 215,375 | 56,940 |
| default | plain_text | 792 | 958 | 1,104 | 216 | 166 | 209 | 120 |
| default | small_json | 6,750 | 8,750 | 3,393 | 1,965 | 5,958 | 7,750 | 3,393 |
| default | truncated_json_4k | 20,542 | 27,042 | 25,853 | 15,064 | 19,584 | 27,125 | 25,741 |
| custom_email | form_encoded | 792 | 1,042 | 1,104 | 216 | 166 | 250 | 120 |
| custom_email | json_4k | 129,500 | 198,208 | 56,643 | 19,622 | 126,458 | 188,500 | 56,643 |
| custom_email | non_matching_json_4k | 129,917 | 215,959 | 56,928 | 19,622 | 128,500 | 326,292 | 56,928 |
| custom_email | plain_text | 791 | 1,042 | 1,104 | 216 | 166 | 209 | 120 |
| custom_email | small_json | 6,000 | 7,875 | 3,393 | 1,957 | 5,000 | 6,625 | 3,393 |
| custom_email | truncated_json_4k | 20,792 | 33,833 | 25,853 | 15,064 | 19,667 | 26,333 | 25,741 |

## Per payload: mask_and_dump (JSON serialization capture path)

Same exact text fixtures passed directly to the real function. This path parses valid object/array JSON; other strings are JSON-quoted/escaped before truncation. It is used by `json_utils.dump_with_context` and by structured-body capture.

| Rules | Payload | Shipped p50 ns | Shipped p99 ns | Peak B | Retained B | Cached p50 ns | Cached p99 ns | Cached peak B |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| default | form_encoded | 1,584 | 2,083 | 1,104 | 726 | 875 | 1,125 | 1,008 |
| default | json_4k | 162,042 | 241,333 | 56,651 | 19,622 | 160,375 | 251,125 | 56,651 |
| default | non_matching_json_4k | 163,125 | 217,250 | 56,940 | 19,622 | 161,500 | 218,709 | 56,940 |
| default | plain_text | 1,542 | 2,041 | 1,104 | 707 | 875 | 1,125 | 1,008 |
| default | small_json | 6,833 | 8,917 | 3,393 | 1,965 | 6,042 | 7,667 | 3,393 |
| default | truncated_json_4k | 28,084 | 58,750 | 28,348 | 19,318 | 27,291 | 55,166 | 28,348 |
| custom_email | form_encoded | 1,542 | 2,000 | 1,104 | 726 | 875 | 1,125 | 1,008 |
| custom_email | json_4k | 128,958 | 211,791 | 56,643 | 19,622 | 126,542 | 185,250 | 56,643 |
| custom_email | non_matching_json_4k | 132,709 | 524,916 | 56,928 | 19,622 | 127,250 | 184,167 | 56,928 |
| custom_email | plain_text | 1,500 | 2,000 | 1,104 | 707 | 875 | 1,125 | 1,008 |
| custom_email | small_json | 6,125 | 7,916 | 3,393 | 1,957 | 5,083 | 6,625 | 3,393 |
| custom_email | truncated_json_4k | 28,250 | 46,875 | 28,348 | 19,318 | 26,792 | 42,709 | 28,348 |

## Per span and invocation

The exact `span_attributes.small_json` and `.json_4k` fixture sets are retained on a minimal recording span. Headers are decoded to mappings once outside timing because Requests already receives header mappings. Each measurement calls real `dump_with_context` for request/response headers and real `add_body_attribute` for request/response body text, then retains all outputs. Other fixture attributes (including `url.full`) are copied unchanged, matching the scope of the Python HTTP client hooks. Body-helper safe-execution and slicing costs and header limit lookup are included. It excludes network calls, Requests machinery, decoding `response.content` bytes, SDK span construction/locking, the enclosing request/response hook safe-execution contexts, and export/protobuf serialization. Thus this measures capture/masking cost for the supplied attribute set, not end-to-end tracing. There is no distro-wide export masking pass. The json_4k span fixture has one 4,111-byte request body and one small response body, exactly as supplied. Invocations execute and retain 20 full captures, rather than multiplying a percentile by 20.

| Rules | Lookup | Fixture | Scope | p50 ns | p99 ns | Peak B | Retained B |
|---|---|---|---|---:|---:|---:|---:|
| default | shipped | small_json | span | 24,833 | 168,875 | 4,567 | 2,586 |
| default | shipped | small_json | invocation20 | 492,083 | 1,228,792 | 22,235 | 20,048 |
| default | shipped | json_4k | span | 178,583 | 257,625 | 57,629 | 20,230 |
| default | shipped | json_4k | invocation20 | 3,682,542 | 5,125,542 | 148,319 | 110,880 |
| default | cached_rule | small_json | span | 20,667 | 29,792 | 4,567 | 2,586 |
| default | cached_rule | small_json | invocation20 | 417,792 | 569,916 | 22,235 | 20,048 |
| default | cached_rule | json_4k | span | 180,833 | 294,542 | 57,629 | 20,230 |
| default | cached_rule | json_4k | invocation20 | 3,612,979 | 8,109,125 | 148,319 | 110,880 |
| custom_email | shipped | small_json | span | 20,208 | 34,125 | 4,445 | 2,583 |
| custom_email | shipped | small_json | invocation20 | 409,084 | 549,125 | 22,120 | 19,988 |
| custom_email | shipped | json_4k | span | 143,667 | 221,000 | 57,627 | 20,235 |
| custom_email | shipped | json_4k | invocation20 | 2,917,938 | 3,222,791 | 148,412 | 110,980 |
| custom_email | cached_rule | small_json | span | 17,209 | 22,208 | 4,445 | 2,583 |
| custom_email | cached_rule | small_json | invocation20 | 350,000 | 450,666 | 22,120 | 19,988 |
| custom_email | cached_rule | json_4k | span | 139,542 | 213,833 | 57,627 | 20,235 |
| custom_email | cached_rule | json_4k | invocation20 | 2,846,958 | 3,800,375 | 148,412 | 110,980 |

## Environment lookup and server query parsing

The query is extracted verbatim from fixture `url.full`: `api_key=k-1&page=2`. `query_parse_and_mask` uses the actual server path: `parse_query_string` followed by `dump_with_context("queryParams", ...)`, including its size-limit environment lookup. It emits a JSON object attribute (`http.request.query`); it does not rewrite the URL. URL extraction itself is outside timing because server hooks already receive a query string.

| Rules | Lookup | Operation | p50 ns | p99 ns | Peak B |
|---|---|---|---:|---:|---:|
| default | shipped | get_rule_only | 708 | 916 | 1,104 |
| default | shipped | query_parse_only | 1,125 | 1,459 | 1,076 |
| default | shipped | query_parse_and_mask | 5,042 | 6,750 | 2,353 |
| default | cached_rule | get_rule_only | 83 | 125 | 120 |
| default | cached_rule | query_parse_only | 1,125 | 1,500 | 1,076 |
| default | cached_rule | query_parse_and_mask | 4,458 | 5,542 | 2,353 |
| custom_email | shipped | get_rule_only | 667 | 916 | 1,104 |
| custom_email | shipped | query_parse_only | 1,125 | 1,459 | 1,076 |
| custom_email | shipped | query_parse_and_mask | 4,875 | 6,292 | 2,352 |
| custom_email | cached_rule | get_rule_only | 83 | 125 | 120 |
| custom_email | cached_rule | query_parse_only | 1,125 | 1,500 | 1,076 |
| custom_email | cached_rule | query_parse_and_mask | 4,125 | 5,416 | 2,352 |

Direct `default` rule-lookup median delta: **625 ns/call** (708 ns shipped vs 83 ns cached). This is the isolated environment lookup/cache-key overhead; payload-level differences also contain scheduling and timing variation.

Direct `custom_email` rule-lookup median delta: **584 ns/call** (667 ns shipped vs 83 ns cached). This is the isolated environment lookup/cache-key overhead; payload-level differences also contain scheduling and timing variation.

## Cold module import and first call

30 fresh CPython processes per rules/payload pair. Only `os`, `sys`, and `time` are explicitly imported before the timer. The import interval includes importing the package and masking dependencies, class/function loading, default regex compilation, and then the first `mask_text` interval includes the first global env-value parsing/regex compilation plus actual masking. Python interpreter/site startup, argument/fixture setup, and process launch are outside the measured interval. Filesystem caches are warm and existing bytecode may be used; these are fresh-process imports, not cold-disk I/O. `DASH0_SWITCH_OFF=true` prevents full tracer/instrumentation activation and `DASH0_DISABLE_PYTHON_DEPENDENCY_CHECK=true` skips dependency scanning, so this isolates import/masking startup and is not full distro cold-start cost.

| Rules | Fixture | Interval | p50 ns | p99 ns |
|---|---|---|---:|---:|
| default | small_json | import_ns | 38,115,604 | 64,326,250 |
| default | small_json | first_call_ns | 72,291 | 161,291 |
| default | small_json | total_ns | 38,191,229 | 64,402,042 |
| default | json_4k | import_ns | 37,620,250 | 41,604,292 |
| default | json_4k | first_call_ns | 246,208 | 462,125 |
| default | json_4k | total_ns | 37,870,625 | 41,872,458 |
| custom_email | small_json | import_ns | 37,726,854 | 42,659,500 |
| custom_email | small_json | first_call_ns | 105,354 | 126,375 |
| custom_email | small_json | total_ns | 37,828,812 | 42,777,084 |
| custom_email | json_4k | import_ns | 37,179,584 | 39,488,458 |
| custom_email | json_4k | first_call_ns | 241,834 | 310,917 |
| custom_email | json_4k | total_ns | 37,416,292 | 39,720,917 |

## Correctness and truncation

| Fixture | Default rules | Custom email rule |
|---|---|---|
| small_json | password/apiToken become `****`; email remains visible | email becomes `****`; password/apiToken remain visible by configured replacement semantics |
| json_4k | password/apiToken masked before budget truncation | email masked before budget truncation; password/apiToken visible |
| truncated_json_4k | Invalid/incomplete JSON: mask_text returns original text, including password/apiToken (**gap**) | Returns original text, including email (**gap**) |
| form_encoded | No form parser on body path: password/csrf_token remain visible (**gap**) | Unchanged; fixture has no email parameter |
| non_matching_json_4k | No default-matching keys; content preserved subject to reserialization/budget truncation | Contains an email key despite fixture name: email correctly becomes `****`, subject to reserialization/budget truncation |
| plain_text | Opaque text returned unchanged, correctly no keyed values | Same |
| URL query through server query parser | JSON query masks api_key; page stays 2 | api_key stays visible because email-only replaces defaults |
| url.full in client span | Untouched by Dash0 masking; api_key remains visible (**gap**) | Untouched; no email key in fixture URL |

For non-JSON strings, `mask_and_dump` quotes/escapes the string but does not hide embedded keyed secrets. For the supplied span: default rules mask request header authorization and response body sessionToken; custom email rules retain both and mask only email in the request body. `set-cookie` is not matched by either supplied rule set. The installed upstream Requests instrumentation calls `redact_url`, whose fixed list is `AWSAccessKeyId`, `Signature`, `sig`, and `X-Goog-Signature`; it does not mask the supplied `api_key`. Known fixture gaps agree with `src/test/unit/masking/test_known_masking_gaps.py` (client URL/query and non-JSON body cases).

Masking happens **before** attribute length truncation. `_parse_json_text` parses the original complete string, `_mask` masks while building an output with an approximate character budget, and `_dump_masked` serializes then slices to max_size and appends `...[too long]` if needed. `add_body_attribute` slices the result again to 4,096 characters; the SDK also has its configured attribute limit. Thus the output can be incomplete JSON even though visible secrets were masked first. If the input is already truncated JSON, parsing fails and no key masking occurs. All raw function and span outputs for both rule sets are saved in `results.json` for inspection.

Source paths: `opt/python/distro/src/dash0_opentelemetry/libs/{masking.py,json_utils.py,general_utils.py}`, `instrumentations/instrumentation_utils.py`, `instrumentations/requests/__init__.py`, `instrumentations/{fastapi/parsers,flask,django}/__init__.py`, and `opt/shared.sh:map_masking_env`.

## Comparison with supplied Rust extension measurements

Provided Criterion release numbers (not remeasured here): small_json 198 ns, json_4k 3,500 ns, truncated_json_4k 3,400 ns, form_encoded 182 ns, non_matching_json_4k 3,300 ns, plain_text 4.5 ns, URL query 235 ns, and 20 spans with 4 KB bodies 94,000 ns. Python capture also parses/serializes and truncates; Rust scans existing attribute strings and has broader coverage (in particular form bodies and truncated JSON). Differences therefore combine runtime overhead, work performed, and correctness coverage. Do not interpret the fast Python no-op paths as equivalent successful masking.

For `default`, Python 20-span capture p50 is **3.683 ms** (39.2× the supplied Rust 94 µs reference), with the scope/coverage differences above.

For `custom_email`, Python 20-span capture p50 is **2.918 ms** (31.0× the supplied Rust 94 µs reference), with the scope/coverage differences above.

# Node.js distro masking benchmark

Measured 2026-10-23 on an Apple M4 Pro (14 cores, 48 GiB RAM), macOS 26.6.2 (25G83), arm64, Node.js v24.21.0. Other workers were running on the laptop (load average 10 to 12 on 14 cores), with no CPU pinning or isolation. The p50 values were stable across three runs, within about 2%. The p99 values are inflated by the load and moved by up to 3× between runs. These are local numbers, not Lambda numbers.

Source: `opt/node` is identical on `master` (`62ee133`) and on this worktree's `HEAD` (`4c276a4`). Packages: `@lumigo/node-core` 1.14.0 and `@opentelemetry/instrumentation-http` 0.219.0. Fixture: `benchmarks/masking/fixtures/masking-payloads.json`, SHA-256 `38c62784…382df7`, used unchanged.

## Results

All times are ns per call. Rust is the extension's criterion number from the brief.

### Per payload

This is `scrubHttpPayload(body, contentType, HTTP_REQUEST_BODY)`, the call every HTTP hook makes for a body. The JSON fixtures use `application/json`, `form_encoded` uses `application/x-www-form-urlencoded`, and `plain_text` uses `text/plain`. The `url_query` row is the upstream `getAbsoluteUrl`, which builds `url.full` on HTTP client spans. The distro does not mask that attribute itself (see Correctness).

| Payload | Default p50 | Default p99 | Custom p50 | Custom p99 | Bytes allocated / call | Rust p50 | Node / Rust (default) |
|---|---:|---:|---:|---:|---:|---:|---:|
| small_json | 5,710 | 14,740 | 3,440 | 8,170 | 6.4 KB | 198 | 29× |
| json_4k | 162,250 | 288,040 | 93,210 | 185,290 | 187 KB | 3,500 | 46× |
| truncated_json_4k | 18,750 | 32,290 | 18,830 | 29,210 | 10.2 KB | 3,400 | 5.5× (does not mask) |
| form_encoded | 399 | 897 | 378 | 952 | 250 B | 182 | 2.2× (does not mask) |
| non_matching_json_4k | 162,040 | 268,290 | 99,380 | 209,420 | 194 KB | 3,300 | 49× |
| plain_text | 367 | 559 | 364 | 524 | 346 B | 4.5 | 80× (does not mask) |
| url_query (upstream) | 386 | 566 | 384 | 566 | 1.5 KB | 235 | 1.6× (fixed list, not DASH0_MASK_*) |

"Default" is the 8 default patterns. "Custom" is `DASH0_MASK_RULES='[".*email.*"]'`, which replaces the defaults, so there is 1 regex instead of 8. The allocation figures come from the default run; the custom run is within 3% of them.

### Per span and per invocation

One HTTP client span from `span_attributes.<name>`, masked the way `instrumentations/https/http.ts` does it at capture time:

- `url.full` through the upstream `getAbsoluteUrl`
- request headers through `CommonUtils.payloadStringify(..., HTTP_REQUEST_HEADERS, 4096)`
- request body through `scrubHttpPayload`
- `HttpHooks.responseHook` for the response headers
- `Http.onRequestEnd` for the response body

The headers are passed as objects, as the hooks receive them, not as the fixture's JSON strings. Attributes the distro never masks (`http.method`, `http.status_code`, `net.peer.name`, `gen_ai.usage.input_tokens`) cost nothing and are not included. One invocation is 20 such spans (`spans_per_invocation`).

| Case | Default p50 | Default p99 | Custom p50 | Custom p99 | Bytes allocated | Rust |
|---|---:|---:|---:|---:|---:|---:|
| span, small_json | 21.8 µs | 35.4 µs | 7.9 µs | 12.7 µs | 15.7 KB | — |
| span, json_4k | 149.7 µs | 238.6 µs | 73.0 µs | 137.5 µs | 212 KB | — |
| invocation (20 spans), small_json | 443 µs | 1,397 µs | 159 µs | 271 µs | 314 KB | — |
| **invocation (20 spans), json_4k** | **3,060 µs** | **3,394 µs** | **1,476 µs** | **1,719 µs** | **4.2 MB** | **94 µs (33× less)** |

The json_4k span measures slightly below the json_4k payload alone. That is run-to-run noise under load; the span cost is about the payload cost plus 15 µs.

The time goes to Lumigo's object walk, not to JSON. A CPU profile of json_4k shows `doScrubObject` at 41% self time, `payloadStringify` at 9%, and regex tests at about 15% (8 patterns per key). Plain `JSON.parse` plus `JSON.stringify` of the same body takes 15.6 µs, against 160 to 216 µs with masking (`bench-scratch/prof.js`).

### Cold cost (fresh process)

Median of 15 fresh processes, with min to max in brackets, in ms:

| Step | Default | Custom |
|---|---:|---:|
| `require('@lumigo/node-core')`: builds 7 scrubbers and their `RegExp` objects | 7.3 (6.6–10.1) | 7.2 (6.7–7.8) |
| `require(payloads)`: adds the distro's `utils`/`logging` and `@opentelemetry/api` | 7.6 (7.0–11.7) | 7.3 (6.7–9.2) |
| first `scrubHttpPayload(small_json)`: interpreter plus lazy regex compile | 0.66 (0.62–0.79) | 0.53 (0.49–0.64) |
| first `scrubHttpPayload(json_4k)` | 0.47 (0.43–0.64) | 0.33 (0.32–0.38) |
| second `scrubHttpPayload(json_4k)` | 0.36 (0.33–0.43) | 0.25 (0.24–0.31) |

The require times are for the unbundled modules loaded from `node_modules`. In the layer, the distro is a webpack bundle that loads at Init in any case, so only Lumigo's share (about 7 ms) is masking-specific. The very first masking call costs about 0.5 to 0.7 ms whatever the payload, which is 115× the warm p50 for small_json. The first json_4k calls cost about 3× the warm p50, and the JIT needs a few hundred calls to reach steady state. The first invocations of a Lambda with 20 spans therefore run mostly un-warmed.

## Correctness per fixture

Default rules, as the request body unless stated otherwise:

| Fixture | Output | Correct? |
|---|---|---|
| small_json | `password` and `apiToken` masked, `email` kept | yes |
| json_4k (4,111 chars) as request body | `password` and `apiToken` masked. Lumigo masks while it walks the object, then cuts at 4,096 chars with `✂` markers (`"qty":"✂"}]}`). The output is still valid JSON | yes. **Masked first, then truncated** |
| json_4k as response body | `http.ts` buffers only the first 4,096 chars, so the JSON is cut before masking. `JSON.parse` fails, which is logged at debug level, and the body is emitted as a quoted string of 4,969 chars. `hunter2`, `t-123` and the email address are all visible. The SDK `attributeValueLengthLimit` then cuts it to 4,096 | **no. Truncated first, so nothing is masked** |
| truncated_json_4k | `JSON.parse` fails, the body is emitted as a quoted string of 4,852 chars, and all secrets are visible. The distro does not use Lumigo's `untruncateJson` | **no** |
| non_matching_json_4k | nothing to mask (default), cut at 4,096 with `✂` | yes |
| form_encoded | emitted as a quoted string. `password=hunter2` and `csrf_token=abc` are visible, because Lumigo never masks top-level strings and the distro does not parse form bodies | **no** |
| plain_text | emitted as a quoted string, unchanged | yes (nothing to mask) |
| request headers | `authorization` masked | yes |
| response headers | nothing matches (`set-cookie` is not a default pattern) | as configured |
| `url.full` | `api_key=k-1` visible. HTTP client spans get only upstream's fixed redaction list (`sig`, `Signature`, `AWSAccessKeyId`, `X-Goog-Signature`), and DASH0_MASK_QUERY_PARAMS does not apply. Lumigo's `HTTP_REQUEST_QUERY` scrubber runs only on express, fastify, kafkajs and amqplib query objects | **no** |

With the custom rule `[".*email.*"]`, the same structural gaps remain (truncated JSON, form bodies, `url.full`). `email` is masked, and `password`, `apiToken` and `authorization` are no longer masked, because custom rules replace the defaults.

## Reproduce

The bench scripts are not in the repo. They are on branch `archive/masking-bench-kits`, under `docs/design/distro-bench/node-bench/`: run `git show origin/archive/masking-bench-kits:docs/design/distro-bench/node-bench/<file>` to get one, or check out that branch. Its `run.sh` runs the real `map_masking_env` from `opt/shared.sh` in a clean `env -i` for each mode, then each script in a fresh `node` process, against the distro compiled with `tsc`.

Method: each case gets a 300 ms warm-up, then 1.5 s of samples. Each sample is a batch of calls sized to take about 20 µs, divided by the batch size. p50 and p99 are therefore percentiles of batch means, not of single calls, so the p99 is somewhat smoothed. "Bytes allocated" is the `heapUsed` delta over K back-to-back calls divided by K, with `--max-semi-space-size=512`, `global.gc()` before the loop, and a GC observer. A sample is discarded and K is reduced whenever any GC runs during the loop. The figure is therefore total bytes allocated per call, not retained bytes.

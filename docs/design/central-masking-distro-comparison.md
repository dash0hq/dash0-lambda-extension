# Masking in the distro or in the extension: cost per language

Should payload masking run in the language distros (today) or in the extension
([`central-masking-plan.md`](central-masking-plan.md))? This compares the real masking code of
each distro with the extension's on the same inputs.

## Conclusion

**Running it in the extension is better on every axis that matters for the function.**

- **Off the response path.** Node and Python mask at capture, inside the handler. Java masks at
  export, but the aws-lambda instrumentation waits for `forceFlush()` before it returns the
  result. All three distros therefore put masking on the caller's latency. The extension masks
  after `platform.runtimeDone`.
- **Cheaper even where it lands.** For one invocation of 20 spans with 4 KB JSON bodies, warm:
  Node 3.06 ms, Python 3.68 ms, Java 0.31 ms (0.10 ms with Java's 1 KB attribute limit),
  extension 0.094 ms. That is 33×, 39× and 3.3× less CPU.
- **Cheap cold.** The extension is compiled ahead of time: 1.0 ms to compile the rules once, then
  0.16 ms for the first 20 spans, after the first response. Python pays 38 ms to import, Node
  about 7 ms. Java pays 21–25 ms on the first invocation and stays at 0.5–3.8 ms per invocation
  for at least the next ten, because the JIT hasn't warmed up.
- **More correct.** It masks truncated JSON, form bodies and URL query strings. Node and Python
  leave all three unmasked; Java misses form bodies.

What it costs: the extension's masking still counts toward billed duration (post-runtime
extension time) and shares the function's vCPU. Under back-to-back load it also delays the next
invocation this sandbox can take: Lambda sends the next event only after the extension calls
`/event/next`. That delay is the extension's masking time, about 0.1 ms warm on this laptop, and
roughly 1.3 ms at 128 MB (~1/14 vCPU). Node's or Python's 3+ ms would sit inside the handler and
scale by the same factor.

## Setup

- Fixtures: `benches/fixtures/masking-payloads.json`, written by the Rust bench
  (`MASKING_FIXTURES_OUT=… cargo bench --bench masking`). Every language used the same bytes
  (SHA-256 `38c62784…382df7`).
- Rules: the 8 default patterns, and the custom rule `DASH0_MASK_RULES='[".*email.*"]'` (which
  replaces the defaults), set the way each distro reads them.
- Machine: Apple M4 Pro, macOS 26.6. The machine was loaded by parallel runs, so treat p99 as
  indicative only. The p50s were stable within a few percent. Lambda CPUs are slower, and a
  128–512 MB function gets 1/14–1/3.5 of a vCPU, so multiply every number by up to ~14 there.
  The ratios hold.
- Code under test:
  - Node: `master`. `scrubHttpPayload` and the `http.ts` hooks, on `@lumigo/node-core` 1.14.0.
  - Python: PR #117. `mask_text` / `mask_and_dump` and the HTTP capture helpers, on
    `lumigo-core` 0.0.16.
  - Java: PR #118. `MaskingSpanExporter` / `PayloadMasker`, on JDK 21.
  - Extension: this PR. `payload_masking.rs`.
- Full reports, with method, raw data and how to reproduce: [`distro-bench/node.md`](distro-bench/node.md),
  [`distro-bench/python.md`](distro-bench/python.md), [`distro-bench/java.md`](distro-bench/java.md).
  The bench kits are next to them.

## Per payload, default rules, warm p50

Body masking as each distro does it for `http.request.body` (Java: `PayloadMasker.maskJson`), in ns.
"✗" means the output still contains the secret.

| Payload | Node | Python | Java | **Extension** | Node / Python / Java vs extension |
|---|---:|---:|---:|---:|---|
| small JSON (116 B) | 5,710 | 6,750 | 688 | **198** | 29× / 34× / 3.5× |
| JSON 4 KB | 162,250 | 162,167 | 14,708 | **3,500** | 46× / 46× / 4.2× |
| JSON cut at 4000 B | 18,750 ✗ | 20,542 ✗ | 14,417 | **3,400** | Node, Python don't mask it |
| form-encoded | 399 ✗ | 833 ✗ | 862 ✗ | **182** | only the extension masks it |
| JSON 4 KB, nothing matches | 162,040 | 162,875 | 13,125 | **3,300** | 49× / 49× / 4.0× |
| plain text | 367 | 792 | 811 | **4.5** | nothing to mask |
| URL query (`url.full`) | 386 ✗ ¹ | 5,042 ² | 497 | **235** | Node and Python don't mask `url.full` |

¹ Upstream `getAbsoluteUrl` with its fixed redaction list; `api_key` stays visible.
² Server-side `http.request.query` parse + mask; client `url.full` is not masked in Python.

The custom rule (1 pattern instead of 8) makes Node and Python 20–45% faster on JSON. It changes
Java by 5–15% and the extension by little: both join the patterns into one regex and cache each
key's result.

Bytes allocated per 4 KB JSON body: Node 187 KB, Python 57 KB peak, Java 23 KB, extension one
output string (0 allocations when nothing matches).

## Per invocation (20 spans), warm p50 and p99

| | Node | Python | Java | **Extension** |
|---|---:|---:|---:|---:|
| 20 × small JSON spans | 443 / 1,397 µs | 492 / 1,229 µs | 36 / 54 µs | **21 µs** |
| 20 × 4 KB JSON spans | 3,060 / 3,394 µs | 3,683 / 5,126 µs | 309 / 1,037 µs (102 µs at Java's 1 KB limit) | **94 µs** |
| same, custom rule | 1,476 µs | 2,918 µs | 290 µs | ~94 µs |

## Cold cost (fresh process)

| | Node | Python | Java | **Extension** |
|---|---|---|---|---|
| Load / compile | ~7 ms to `require` Lumigo (at Init) | ~38 ms to import (at Init) | 11–14 ms class load + `Pattern.compile` | **1.0 ms** rule compile, on the first export |
| First masking call | 0.5–0.7 ms per call; JIT needs a few hundred calls | 0.07–0.25 ms | 5–15 ms for the first 20-span export | **0.16 ms** for the first 20 spans |
| Next invocations | interpreted, roughly 1–3× warm | warm from the first call | 0.5–3.8 ms per invocation for the next 10+ | warm from the first call |
| Lands | Init, then in the handler | Init, then in the handler | first `forceFlush`, which the result waits on | after the first response |

The extension number comes after this change: the six rule sets (one per variable) used to be
compiled separately, which cost 4.7 ms; identical lists are now compiled once.

## Where the cost lands

| | When it runs | On the caller's response path? | Billed? | Delays the next invocation on this sandbox? |
|---|---|---|---|---|
| Node | at capture, inside the handler | **yes** | yes | yes (the handler is longer) |
| Python | at capture, inside the handler | **yes** | yes | yes |
| Java | at export; the aws-lambda instrumentation waits for `forceFlush` before returning | **yes** | yes | yes |
| Extension, `DASH0_SEND_ON_INVOCATION_END=true` (default) | after `platform.runtimeDone`, before `/event/next` | no | yes (post-runtime extension duration) | yes, by its masking time (~0.1 ms warm here) |
| Extension, `=false` | at the next `INVOKE`, concurrently with the next handler | no | yes | competes for the next handler's vCPU |

The e2e harness (`tests/masking_e2e.rs`, release) confirms this. The `/v1/traces` ack the
handler waits on is the same with and without extension masking (p50 697 vs 688 µs). The time
from `runtimeDone` to `/event/next` grows by about 60 µs p50.

## Secondary effects of moving masking out of the distros

- **Payload size over localhost: unchanged.** Masking replaces values with `****`. Twenty spans
  are 93,800 B unmasked and 93,620 B masked (0.2%); with small bodies, 13,858 vs 13,678 B.
  Serializing and sending unmasked data costs the distro the same as masked data. Today the
  distro also pays the masking itself (and Node and Python re-serialize the body).
- **Node truncation before masking (correctness).** Node's `http.ts` buffers only the first 4,096
  characters of a response body. A large JSON response is therefore cut before masking, fails
  `JSON.parse`, and is sent as a string literal with every secret visible. The extension unwraps
  that literal and masks the cut JSON. Python masks first and truncates after, but still leaves
  input that was already truncated unmasked. Java truncates first (1,024 chars by default) and
  repairs the JSON before masking.
- **Distros would still parse and stringify bodies** for capture and truncation. Node should keep
  calling Lumigo with empty rules so it truncates structurally. The Lumigo walk costs about 10×
  a plain `JSON.parse` + `JSON.stringify` of the same body (15.6 µs vs 160 µs), so dropping the
  rules removes most of the cost, though that wasn't measured separately.
- **Coverage gaps closed by the extension:** form bodies (all three distros), truncated JSON
  (Node, Python), `url.full` query values (Node, Python), and Java's attributes beyond its
  exporter.

## Reproduce

```sh
# extension (this repo)
MASKING_FIXTURES_OUT=$PWD/benches/fixtures/masking-payloads.json cargo bench --bench masking
cargo test --release --test masking_e2e -- --nocapture
# distros: see "Reproduce" in distro-bench/{node,python,java}.md; kits in distro-bench/*-bench/
```

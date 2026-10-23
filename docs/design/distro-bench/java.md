# Java distro masking cost

What it costs to mask payloads inside the Java distro (PR #118, branch `RaphaelManke/sig-701-java-masking`, commit `f79a372`), measured on the shared fixtures `benches/fixtures/masking-payloads.json`. The code under test is `MaskingSpanExporter`, which wraps the OTLP span exporter. It masks each span at export time through `PayloadMasker` (Jackson streaming parser for bodies, a split on `&` for query strings) and `MaskingRules` (all patterns joined into one case-insensitive regex, plus a per-key result cache).

## Machine and method

- Apple M4 Pro, 14 cores, 48 GB, macOS 26.6.2, on AC power. Lambda vCPUs are slower, and a 128–512 MB function gets only a fraction of one, so expect the cold numbers to be several times higher there.
- JDK: Zulu 21.0.6 (`OpenJDK 64-Bit Server VM 21.0.6+7-LTS`), default flags, `-Xms512m` for the warm runs.
- Harness: a plain `System.nanoTime` loop, not JMH. Each case runs 200k warm-up calls, then 5000 samples. Each sample times a batch of *k* calls, with *k* picked so a sample takes about 20 µs, because Apple Silicon's `nanoTime` ticks every ~42 ns. p50 and p99 are taken across the 5000 per-call averages. Calls of 20 µs or more (spans with 4 KB bodies, invocations) are timed one call per sample. Allocations come from `ThreadMXBean.getThreadAllocatedBytes`, averaged per call.
- Rule sets: **defaults** (no config, `MaskingRules.DEFAULT_PATTERNS`) and **custom** (`DASH0_MASK_RULES='[".*email.*"]'`, passed as the `dash0.mask.rules` config property, which is how the agent reads the env var. The wrapper also maps it to `LUMIGO_SECRET_MASKING_REGEX`, but the Java distro reads `DASH0_MASK_*`). As in the extension, the custom rule *replaces* the defaults.
- Spans: built with the real OTel SDK (`SdkTracerProvider`, CLIENT span), so the SDK attribute length limit applies exactly as in the distro. The fixture span carries `http.request.headers` / `http.response.headers` as JSON strings (Node.js style). The Java instrumentations capture headers as one `http.request.header.<name>` / `http.response.header.<name>` string-array attribute per header, so the bench converts them to that form. All other fixture attributes are used as they are.
- "Per payload" calls the masking function the exporter uses for that attribute: `PayloadMasker.maskJson` with the request-body rules for each payload (as if it were `http.request.body`), and `PayloadMasker.maskUrl` with the query-param rules for `url.full`.
- "Per span" is `MaskingSpanExporter.mask(span)` on one span. "Per invocation" is `MaskingSpanExporter.export(20 spans)` with a no-op delegate, so it covers masking only and excludes OTLP serialization.

## Results: JIT-warm

Defaults (`DASH0_MASK_RULES` unset):

| case | p50 ns | p99 ns | bytes/op | Rust ext p50 |
|---|---:|---:|---:|---:|
| payload `small_json` | 688 | 9,618¹ | 1,568 | 198 ns |
| payload `json_4k` | 14,708 | 53,291¹ | 23,160 | 3.5 µs |
| payload `truncated_json_4k` | 14,417 | 21,042 | 23,456 | 3.4 µs |
| payload `form_encoded` | 862 | 1,095 | 2,512 | 182 ns |
| payload `non_matching_json_4k` | 13,125 | 17,417 | 19,176 | 3.3 µs |
| payload `plain_text` | 811 | 1,212 | 2,512 | 4.5 ns |
| `url.full` query | 497 | 793 | 790 | 235 ns |
| **span** `small_json` (distro limit 1024) | 1,742 | 2,604 | 5,784 | |
| **span** `json_4k` (distro limit 1024) | 5,069 | 7,500 | 11,520 | |
| **span** `json_4k` (no length limit) | 14,833 | 21,084 | 27,352 | |
| **invocation** 20 × `small_json` (limit 1024) | 35,791 | 54,417 | 115,776 | |
| **invocation** 20 × `json_4k` (limit 1024) | 102,167 | 146,167 | 230,496 | |
| **invocation** 20 × `json_4k` (no length limit) | 308,750 | 1,036,958¹ | 547,136 | 94 µs |

Custom rule `[".*email.*"]`:

| case | p50 ns | p99 ns | bytes/op |
|---|---:|---:|---:|
| payload `small_json` | 672 | 1,262 | 1,600 |
| payload `json_4k` | 12,958 | 17,750 | 23,192 |
| payload `truncated_json_4k` | 13,000 | 18,875 | 23,488 |
| payload `form_encoded` | 844 | 1,112 | 2,512 |
| payload `non_matching_json_4k` | 13,125 | 21,167 | 23,272 |
| payload `plain_text` | 816 | 1,116 | 2,512 |
| `url.full` query | 371 | 495 | 476 |
| **span** `small_json` (limit 1024) | 1,486 | 2,181 | 5,088 |
| **span** `json_4k` (limit 1024) | 4,930 | 7,945 | 10,824 |
| **span** `json_4k` (no length limit) | 14,208 | 25,667 | 26,656 |
| **invocation** 20 × `small_json` (limit 1024) | 30,541 | 45,250 | 101,856 |
| **invocation** 20 × `json_4k` (limit 1024) | 99,458 | 160,875 | 216,576 |
| **invocation** 20 × `json_4k` (no length limit) | 290,042 | 356,875 | 533,216 |

¹ Single GC pauses landed in these samples. In an earlier identical run, the same rows had p99 1,260 / 18,791 / 407,459 ns. The p50 values matched within 10 %.

Takeaways (warm):

- Per JSON payload, the Java distro is about 3.5–4× slower than the Rust scanner (14 µs vs 3.5 µs for 4 KB) and allocates about 5.6× the body size (23 KB per 4 KB body). Jackson re-serializes the whole body, even when nothing matches.
- Non-JSON bodies (`form_encoded`, `plain_text`) still cost about 0.8 µs and 2.5 KB each, because a parser and generator are created before the first token shows the body isn't JSON. Rust skips plain text in 4.5 ns.
- With the distro's default `otel.span.attribute.value.length.limit=1024`, a 4 KB body is cut to 1 KB before masking, so the realistic 20-span invocation costs about 100 µs. That is about the same as the extension's 94 µs for full 4 KB bodies. Without the limit (bodies the size the extension sees), it costs about 300 µs, roughly 3.3× the extension.
- The custom rule (one pattern) is only 5–15 % cheaper than the 8 defaults: the patterns are joined into one regex and each key's result is cached, so parsing and writing JSON dominate, not regex matching.

## Results: cold (fresh JVM per run, 5 runs each, median shown)

Each run starts a new JVM. It first builds the 20 SpanData objects with the SDK and runs one export through the no-op delegate, so the SDK classes are already loaded, as they would be in the agent. Only the masking classes, Jackson, and the regex compile are left cold. "Config" is `MaskingConfig.fromConfig` plus `new MaskingSpanExporter` (class load + `Pattern.compile`). "First export" is the first `export()` of 20 spans. Then 10 more exports follow, alternating between two batches.

| rules | span set | config + compile | first export (20 spans) | exports 2–11 (µs, in order) | alloc config / first export |
|---|---|---:|---:|---|---:|
| defaults | `small_json` | 10.8 ms | 10.7 ms | 1749, 1436, 1285, 1112, 970, 970, 767, 677, 601, 590 | 2.1 MB / 856 KB |
| defaults | `json_4k` (limit 1024) | 10.7 ms | 14.6 ms | 3788, 2284, 1609, 1296, 1266, 1272, 1145, 1126, 997, 964 | 2.1 MB / 995 KB |
| custom | `small_json` | 13.8 ms | 4.7 ms | 1664, 1288, 1243, 1010, 911, 833, 670, 661, 513, 503 | 2.4 MB / 308 KB |
| custom | `json_4k` (limit 1024) | 13.7 ms | 7.1 ms | 2496, 1881, 1430, 1110, 1006, 1031, 981, 891, 862, 897 | 2.4 MB / 447 KB |

(Exports 2–11 show the median run; the 5 runs varied by about ±20 %. Full raw output is in `java-bench/cold.txt`.)

- The **first invocation pays about 21–25 ms** for masking on this laptop: about 11–14 ms to load the classes and compile the patterns, plus 5–15 ms for the first export. Custom-rule config is slower and its first export faster only because `MaskingRules.parse` loads Jackson during config instead of during the first export. The totals are similar.
- **The JIT doesn't warm up within a typical Lambda container lifetime.** Exports 2–11 still take 0.5–3.8 ms, 10–40× the warm 36–102 µs. In the interpreter or C1, masking 20 spans costs about 1 ms per invocation for the first dozen invocations, and Lambda's fractional vCPU makes this worse.
- The Rust extension has no JIT warm-up: its warm numbers apply from the first invocation. Its pattern-compile cost was not measured here.

## Masking before or after truncation

**After.** The OTel SDK enforces `otel.span.attribute.value.length.limit` (set to 1024 by `Dash0Configurator` unless the user overrides it) when the attribute is set on the span. `MaskingSpanExporter` runs at export, so it only ever sees the truncated value. `PayloadMasker.maskJson` is written for that: it masks the valid prefix, drops the value that was cut off, and closes the open objects and arrays so the result is valid JSON again.

## Correctness per fixture

Full output is in `java-bench/correctness.txt`.

| fixture | defaults | custom `.*email.*` |
|---|---|---|
| `small_json` | ✅ `password`, `apiToken` → `****`. `email` kept. | ✅ `email` → `****`. `password`, `apiToken` left as they are, because the custom rule replaces the defaults. |
| `json_4k` | ✅ same keys masked, the rest unchanged (re-serialized compact; length 4111 → 4107) | ✅ `email` masked |
| `truncated_json_4k` | ✅ **masked, not left alone**: `password`/`apiToken` → `****`. The cut-off last item becomes `{}` and the output is closed as valid JSON (`…{}]}`). This differs from an extension that leaves invalid JSON unmasked. | ✅ `email` masked, same repair |
| `form_encoded` | ❌ **not masked**: `password=hunter2` and `csrf_token=abc` pass through. Bodies are only masked when they are JSON; form bodies are left as they are (only `url.full`/`url.query` get query-style masking). | ❌ not masked (would need the same fix) |
| `non_matching_json_4k` | ✅ unchanged, and the original string is returned as is, with no copy | ✅ `email` masked |
| `plain_text` | ✅ unchanged, as expected (only `all` masks non-JSON) | ✅ unchanged |
| span `url.full` | ✅ `api_key=****&page=2` | ✅ unchanged (`api_key` doesn't match `email`) |
| span headers (as `http.*.header.<name>`) | ✅ `authorization` → `[****]`, others kept | ✅ nothing matches, all kept |
| span `http.response.body` | ✅ `sessionToken` → `****` | ✅ unchanged |
| span `gen_ai.usage.input_tokens` | kept: not a payload attribute, so not masked even though the name contains `token` | kept |

## Reproduce

The bench kit is in `docs/design/distro-bench/java-bench/` (untracked): `DistroBench.java`, `cp.gradle`, `fixtures-to-properties.py`, `run.sh`, and the raw outputs. The fixtures are converted to a base64 `.properties` file so the cold run doesn't load Jackson before the masking code does.

```sh
/Users/rm/code/dash0/dash0-claude/central-masking-extension/docs/design/distro-bench/java-bench/run.sh \
  /Users/rm/code/dash0/dash0-claude/sig-701-java-masking
```

`run.sh` copies `DistroBench.java` into `custom/src/test/java/io/dash0/javaagent/masking/` (the same package, so it can call the package-private `mask()` and `PayloadMasker`), runs `./gradlew -I cp.gradle :custom:printBenchCp` to compile the test classes and print the runtime classpath, and runs `java -cp … DistroBench <fixtures> correctness|warm|cold <defaults|custom> [span set]`. On exit it deletes the copied source and its class files, so the worktree stays clean. It takes about 2 minutes.

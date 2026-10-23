# Masking in the distro or in the extension: real Lambda, per runtime

What does a function gain end to end when payload masking moves from the language distro into
the extension (`DASH0_MASK_IN_EXTENSION=true`, [`central-masking-plan.md`](central-masking-plan.md))?
Measured on AWS Lambda, arm64, us-west-2, on 2026-10-23. The laptop numbers are in
[`central-masking-distro-comparison.md`](central-masking-distro-comparison.md).

## Answer

**The gain is real only where the CPU is scarce: Node.js and Python at 128 MB.** At 512 MB and
1024 MB, and for Java at any size, it is below the noise of this setup (about ±5 ms warm).

| Runtime, memory | Response latency, warm p50 | Response latency, cold p50 | Billed, warm p50 | Extension time after the response, cold p50 |
|---|---:|---:|---:|---:|
| Node.js 22, 128 MB | **−54 ms** (−27 to −78) | **−145 ms** (−47 to −214) | **−49 ms** | **+80 ms** |
| Node.js 22, 512 MB | noise (−4) | −30 ms (−1 to −60) ¹ | −12 ms ¹ | +10 ms |
| Node.js 22, 1024 MB | −4 ms ¹ | noise | −9 ms ¹ | +6 ms ¹ |
| Python 3.13, 128 MB | **−19 ms** (−14 to −22) | noise | **−21 ms** | **+64 ms** |
| Python 3.13, 512 MB | noise (+5) ¹ | noise | noise (+8) ¹ | +15 ms |
| Python 3.13, 1024 MB | noise | noise | noise | +4 ms |
| Java 21, 512 MB | noise | noise (±200 ms) | noise | +13 ms |
| Java 21, 1024 MB | noise | noise | noise | +6 ms |
| Java 21, 128 MB | not measurable: every invocation with the layer times out (see below) | | | |

`extension-masking` minus `distro-masking`, p50, with the bootstrap 95% interval where it matters.
Negative is better. "Response latency" is Lambda's `responseLatency`: from the invoke to the
runtime posting the response, which is what the caller waits for.
¹ Inside the noise floor measured below, even though the interval excludes 0.

- **The caller gets the response sooner.** Node.js at 128 MB spends 39 ms per warm invocation
  masking in the handler (`distro` − `none`), and about 155 ms on the first invocation of a
  sandbox. Python at 128 MB spends 24 ms. Moving masking out removes it from the response path.
- **The extension pays it back after the response, but only cold.** On the first invocation the
  extension takes 64–80 ms longer at 128 MB, most likely compiling the rules and the first
  masking pass on 1/14 of a vCPU (not broken down further). Warm, its extra time is within ±4 ms. This time is billed and delays the next
  event on the same sandbox, not the caller.
- **Billed duration falls** where the distro's cost was large (Node.js −49 ms, Python −21 ms per
  warm invocation at 128 MB); elsewhere the change is within noise. Cold billed duration varies by
  ±200 ms between sandboxes, so no cold delta is resolvable.
- **Init duration and memory don't change.** In this setup the distros still load their masking
  code (they run with empty rules), so the init saving of not loading Lumigo is not measured.
- **Java gains nothing measurable.** Its exporter masks attributes it has already truncated to
  1 KB, which costs well under a millisecond warm.

## Setup

- **One layer per runtime**, the same in every variant: the extension binary from this branch
  (`7e343be`, `aarch64-unknown-linux-musl`) and the distro from `master` (Node.js), PR #117
  (Python, `abcc83f`) or PR #118 (Java, `f79a372`). With `DASH0_MASK_IN_EXTENSION` unset the
  extension behaves like `master`. Variants differ only in environment variables, so layer
  content is not a factor.

  | Variant | Distro masking | Extension masking | Environment |
  |---|---|---|---|
  | `baseline` | – | – | no layer |
  | `distro` | default rules | off | layer defaults |
  | `ext` | off (empty rules) | default rules | `DASH0_MASK_IN_EXTENSION=true`; Node.js and Python `LUMIGO_SECRET_MASKING_REGEX='[]'`; Java `JAVA_TOOL_OPTIONS=-Ddash0.mask.rules=[]` |
  | `none` | off | off | the distro switch only |

  PR #122 as it stands keeps the distros masking when the flag is on (`distro` + `ext`). That
  combination wasn't run: it would cost what `distro` costs on the response path, plus the
  extension's time after it.
- **Workload** (`benchmarks/iac/lambdas/masking`, Java in `MaskingWorkloadHandler.java`): a JSON
  event with a `password`, then five sequential HTTP POSTs of the same 4,018-byte JSON body
  (`password`, `api_key`, `access_token` and 41 items; byte-identical in all three runtimes) to
  an echo Function URL, which returns the body. Node.js uses `https.request`, Python `requests`,
  Java `java.net.http.HttpClient`: the clients whose bodies each distro captures. The bodies stay
  under Node.js's 4,096-character capture limit so that Node.js masks them instead of sending a
  cut, unparsable string.
- **Sink**: the same Function URL (Node.js 22, 1024 MB) is the extension's `DASH0_ENDPOINT`. It
  decompresses each OTLP request and logs how many secret values and `****` it contains. No
  telemetry left the account.
- **Runs**: 128, 512 and 1024 MB; per function 10 cold starts (forced by an environment-variable
  change) and 50 sequential warm invocations after one discarded warm-up. The four variants of a
  runtime and memory size ran at the same time, 9 functions at a time.
- **Metrics** per invocation from the JSON platform logs: `platform.report` (`initDurationMs`,
  `durationMs`, `billedDurationMs`, `maxMemoryUsedMB`, the `extensionOverhead` span) and
  `platform.runtimeDone` (`responseLatency` and `runtimeOverhead` spans). The client-side
  `Invoke` time was recorded too, from a laptop in Germany.

## Masking was verified

Per function at 512 MB, after one invocation (sink log):

| | `distro` | `ext` | `none` |
|---|---|---|---|
| `/v1/logs` (HTTP bodies, moved off the spans by the extension), secrets that arrived | 0 | 0 | 30 of 30 |
| `/v1/traces`, secrets that arrived | 0 | 0 | 0 |

The same in all three runtimes. 30 is 5 calls × request and response × 3 keys. The event's
`password` is masked by the extension in every variant, as before.

## Noise

- `ext` and `none` run the same handler code (the distros mask with empty rules in both), so
  their difference in response latency is the noise floor: up to ±5 ms warm at 512 MB and
  1024 MB, ±15 ms warm for Node.js at 128 MB, ±10–30 ms cold for Node.js and Python, and
  ±200 ms cold for Java. Python at 512 MB shows `distro` 5 ms *faster* than `none`, with an
  interval that excludes 0: that is host-to-host variance, which the bootstrap interval (sampling
  within one sandbox) doesn't cover.
- **Client latency is not usable for the deltas.** It includes about 270 ms of round trip from
  Europe, and its p50 shifts by up to 45 ms between functions with the same Lambda-side numbers
  (Python 512 MB warm: `distro` 349 ms, `ext` 396 ms, response latency 82 vs 86 ms). It is kept
  in the tables for completeness.
- `extensionOverhead` is about 90 ms in every layer variant, mostly the export to the sink
  Function URL (TLS and a Lambda invoke per request). Against a real Dash0 endpoint it differs;
  the masking part of it is the cold delta above.
- `runtimeOverhead` (from the response to the runtime's next `/next`) is 20–40 ms at 128 MB with
  or without the layer. It's not related to masking.

## Java at 128 MB

Java 21 with the Dash0 agent does not run at 128 MB: all 180 invocations of `distro`, `ext` and
`none` timed out at 60 s with 127 MB of 128 MB used, cold and warm. Without the layer it runs
(init 648 ms, first invocation 8 s). The timeout was lowered to 5 s mid-run to finish faster, so
those runs record errors only.

## Results

p50 / p90 / p99 in ms. `n` counts successful invocations. Raw data:
[`benchmarks/results/2026-10-23-masking-aws.json`](../../benchmarks/results/2026-10-23-masking-aws.json).


### node22, 128 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 128.2 / 153.3 / 173.0 | 1271.1 / 1327.1 / 1330.6 | 716.3 / 755.3 / 806.0 | 1.1 / 20.0 / 20.0 | – | 736.1 / 757.2 / 807.6 | 867.0 / 906.0 / 936.0 | 79 |
| distro | 10 | 606.6 / 788.3 / 801.5 | 2519.8 / 2709.6 / 2762.4 | 1400.0 / 1471.5 / 1495.0 | 20.1 / 39.4 / 40.0 | 238.2 / 253.8 / 258.5 | 1669.3 / 1729.6 / 1766.5 | 2286.0 / 2518.0 / 2568.0 | 115 |
| ext | 10 | 609.6 / 769.0 / 800.5 | 2324.7 / 2558.7 / 2592.4 | 1258.4 / 1330.4 / 1355.0 | 20.4 / 39.3 / 39.9 | 318.5 / 337.2 / 340.8 | 1595.3 / 1711.9 / 1712.1 | 2248.0 / 2329.0 / 2513.0 | 114 |
| none | 10 | 582.2 / 630.6 / 654.7 | 2277.9 / 2437.0 / 2564.9 | 1243.9 / 1327.5 / 1441.5 | 21.3 / 39.5 / 39.8 | 225.4 / 254.2 / 255.5 | 1502.8 / 1565.1 / 1718.3 | 2092.0 / 2196.0 / 2374.0 | 115 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 364.3 / 382.4 / 436.3 | 81.5 / 96.8 / 162.2 | 0.3 / 8.7 / 39.4 | – | 84.3 / 101.1 / 202.0 | 85.0 / 102.0 / 202.0 | 89 |
| distro | 50 | 561.4 / 687.4 / 1023.6 | 288.2 / 396.6 / 742.1 | 20.5 / 39.3 / 40.0 | 91.9 / 102.9 / 192.2 | 418.4 / 518.3 / 954.7 | 419.0 / 519.0 / 955.0 | 117 |
| ext | 50 | 552.1 / 667.4 / 1113.7 | 238.4 / 367.4 / 792.4 | 19.8 / 39.9 / 119.2 | 95.6 / 106.1 / 111.9 | 367.5 / 509.4 / 925.8 | 368.0 / 510.0 / 926.0 | 117 |
| none | 50 | 577.8 / 680.1 / 992.9 | 253.0 / 341.9 / 655.6 | 20.0 / 39.8 / 59.3 | 93.1 / 100.1 / 113.1 | 381.7 / 466.4 / 768.8 | 382.0 / 467.0 / 769.0 | 115 |

### node22, 512 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 125.7 / 161.4 / 167.7 | 731.6 / 801.0 / 835.8 | 184.4 / 197.0 / 211.3 | 0.7 / 0.8 / 0.9 | – | 185.8 / 198.3 / 212.7 | 319.0 / 355.0 / 381.0 | 81 |
| distro | 10 | 625.4 / 803.8 / 812.3 | 1419.6 / 1578.3 / 1650.3 | 332.2 / 354.3 / 354.5 | 1.4 / 1.9 / 15.6 | 84.8 / 96.6 / 105.3 | 417.6 / 455.6 / 458.4 | 1043.0 / 1248.0 / 1271.0 | 135 |
| ext | 10 | 611.9 / 783.2 / 788.5 | 1385.2 / 1546.9 / 1591.3 | 308.9 / 330.4 / 334.0 | 1.7 / 18.4 / 18.8 | 95.4 / 109.5 / 112.2 | 408.9 / 430.9 / 445.4 | 1030.0 / 1201.0 / 1234.0 | 135 |
| none | 10 | 610.6 / 799.5 / 814.4 | 1379.9 / 1591.2 / 1735.0 | 310.2 / 331.6 / 336.3 | 1.3 / 1.6 / 1.9 | 83.7 / 94.3 / 99.0 | 395.8 / 419.7 / 421.0 | 1015.0 / 1221.0 / 1235.0 | 135 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 343.7 / 358.5 / 376.2 | 73.7 / 80.9 / 91.8 | 0.3 / 0.4 / 5.9 | – | 74.4 / 81.7 / 92.5 | 75.0 / 82.0 / 93.0 | 91 |
| distro | 50 | 396.8 / 408.7 / 443.3 | 84.6 / 103.2 / 141.7 | 1.0 / 18.5 / 20.6 | 92.8 / 98.3 / 106.9 | 189.1 / 203.2 / 251.0 | 190.0 / 204.0 / 252.0 | 146 |
| ext | 50 | 349.8 / 383.9 / 403.6 | 80.6 / 99.1 / 117.8 | 0.8 / 10.6 / 18.1 | 91.8 / 98.7 / 130.8 | 176.9 / 198.2 / 229.6 | 177.0 / 199.0 / 230.0 | 143 |
| none | 50 | 395.8 / 427.6 / 511.4 | 83.5 / 106.3 / 209.2 | 1.1 / 16.9 / 21.9 | 92.3 / 97.6 / 102.5 | 185.3 / 202.1 / 322.5 | 186.0 / 203.0 / 323.0 | 143 |

### node22, 1024 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 125.8 / 131.2 / 137.9 | 645.8 / 700.0 / 744.2 | 117.8 / 125.7 / 131.9 | 0.6 / 0.8 / 0.9 | – | 119.0 / 127.0 / 133.5 | 244.0 / 265.0 / 265.0 | 81 |
| distro | 10 | 612.4 / 648.9 / 788.4 | 1218.5 / 1290.4 / 1425.0 | 182.8 / 194.0 / 219.4 | 1.2 / 1.4 / 1.5 | 87.5 / 91.4 / 91.7 | 268.7 / 285.3 / 312.8 | 893.0 / 918.0 / 1102.0 | 135 |
| ext | 10 | 603.7 / 788.9 / 795.4 | 1192.1 / 1452.4 / 1453.2 | 181.1 / 199.7 / 200.9 | 1.3 / 1.5 / 1.5 | 92.7 / 96.4 / 99.1 | 272.6 / 296.5 / 302.2 | 875.0 / 1063.0 / 1092.0 | 136 |
| none | 10 | 586.0 / 618.8 / 802.3 | 1159.8 / 1377.2 / 1427.4 | 170.1 / 195.9 / 198.0 | 1.2 / 1.5 / 1.5 | 85.0 / 87.7 / 89.9 | 257.0 / 283.3 / 290.1 | 842.0 / 903.0 / 1093.0 | 135 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 384.4 / 397.3 / 420.9 | 74.2 / 80.3 / 96.4 | 0.3 / 0.4 / 1.3 | – | 74.8 / 80.9 / 97.1 | 75.0 / 81.0 / 98.0 | 91 |
| distro | 50 | 398.3 / 417.6 / 459.0 | 87.0 / 95.8 / 146.9 | 0.7 / 0.9 / 13.3 | 96.7 / 104.0 / 107.9 | 187.4 / 200.2 / 239.3 | 188.0 / 201.0 / 240.0 | 144 |
| ext | 50 | 354.9 / 367.5 / 371.6 | 82.7 / 92.3 / 111.5 | 0.9 / 1.0 / 1.4 | 94.4 / 101.8 / 108.8 | 178.7 / 192.6 / 204.9 | 179.0 / 193.0 / 205.0 | 147 |
| none | 50 | 352.7 / 369.9 / 401.1 | 79.6 / 88.6 / 115.4 | 0.8 / 1.0 / 5.5 | 94.1 / 99.0 / 105.7 | 176.0 / 184.7 / 205.8 | 176.0 / 185.0 / 206.0 | 144 |

### python313, 128 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 334.0 / 403.2 / 422.0 | 938.6 / 996.6 / 1063.5 | 185.4 / 198.2 / 214.9 | 18.9 / 19.3 / 19.3 | – | 205.4 / 217.9 / 228.9 | 541.0 / 602.0 / 640.0 | 67 |
| distro | 10 | 1108.7 / 1391.6 / 1460.2 | 2056.3 / 2385.1 / 2444.5 | 390.0 / 413.7 / 444.7 | 20.0 / 39.2 / 39.5 | 191.1 / 212.5 / 214.8 | 610.3 / 634.9 / 680.2 | 1735.0 / 1989.0 / 2094.0 | 102 |
| ext | 10 | 1097.3 / 1192.0 / 1484.8 | 1942.0 / 2139.5 / 2384.1 | 386.1 / 433.0 / 438.5 | 20.4 / 39.5 / 40.1 | 259.3 / 302.6 / 339.1 | 676.4 / 751.9 / 813.2 | 1770.0 / 1923.0 / 2298.0 | 102 |
| none | 10 | 1110.8 / 1361.5 / 1379.9 | 2036.1 / 2262.5 / 2413.3 | 388.3 / 415.9 / 437.0 | 39.2 / 39.4 / 39.6 | 195.0 / 215.3 / 218.8 | 615.3 / 650.8 / 676.2 | 1747.0 / 1964.0 / 1987.0 | 102 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 371.3 / 382.6 / 398.9 | 94.5 / 102.7 / 120.4 | 19.3 / 19.5 / 19.6 | – | 111.2 / 120.3 / 135.0 | 112.0 / 121.0 / 135.0 | 67 |
| distro | 50 | 570.0 / 586.9 / 635.0 | 256.9 / 276.9 / 279.8 | 39.4 / 39.8 / 39.9 | 67.0 / 76.3 / 94.5 | 354.9 / 371.7 / 388.8 | 355.0 / 372.0 / 389.0 | 104 |
| ext | 50 | 545.7 / 579.8 / 616.6 | 237.1 / 260.4 / 293.2 | 39.2 / 39.7 / 39.8 | 66.5 / 79.4 / 98.2 | 333.1 / 360.9 / 399.1 | 334.0 / 361.0 / 400.0 | 102 |
| none | 50 | 504.8 / 518.8 / 554.6 | 232.4 / 241.5 / 280.3 | 19.9 / 39.7 / 39.9 | 69.6 / 81.3 / 99.2 | 327.9 / 345.6 / 359.3 | 328.0 / 346.0 / 360.0 | 100 |

### python313, 512 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 347.3 / 426.0 / 441.9 | 850.0 / 939.5 / 1007.9 | 94.4 / 98.6 / 102.2 | 0.5 / 0.7 / 11.6 | – | 97.6 / 103.3 / 106.7 | 434.0 / 526.0 / 533.0 | 67 |
| distro | 10 | 1119.5 / 1333.0 / 1339.1 | 1728.7 / 1951.9 / 1986.9 | 105.0 / 111.9 / 120.2 | 0.9 / 6.4 / 13.3 | 83.2 / 88.2 / 91.2 | 188.9 / 202.1 / 203.4 | 1309.0 / 1522.0 / 1526.0 | 111 |
| ext | 10 | 1072.3 / 1213.0 / 1404.2 | 1644.0 / 1793.6 / 2014.3 | 96.5 / 119.2 / 126.8 | 0.9 / 14.2 / 17.4 | 95.8 / 106.9 / 110.3 | 202.1 / 222.4 / 228.8 | 1294.0 / 1409.0 / 1634.0 | 111 |
| none | 10 | 1076.1 / 1114.0 / 1352.3 | 1630.1 / 1765.1 / 1974.9 | 96.8 / 105.1 / 108.8 | 1.1 / 7.6 / 15.2 | 83.0 / 89.1 / 92.6 | 180.0 / 198.8 / 199.7 | 1267.0 / 1314.0 / 1531.0 | 111 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 354.0 / 369.6 / 406.8 | 84.2 / 93.0 / 107.7 | 0.4 / 0.5 / 0.6 | – | 85.0 / 93.8 / 108.5 | 85.0 / 94.0 / 109.0 | 71 |
| distro | 50 | 348.6 / 362.3 / 369.7 | 81.5 / 89.4 / 93.2 | 0.9 / 1.0 / 1.3 | 91.4 / 96.1 / 100.8 | 174.4 / 184.1 / 190.0 | 175.0 / 185.0 / 191.0 | 113 |
| ext | 50 | 395.7 / 410.3 / 433.7 | 86.2 / 96.0 / 103.3 | 0.9 / 1.3 / 14.4 | 92.7 / 98.9 / 103.2 | 181.8 / 192.4 / 203.0 | 182.0 / 193.0 / 204.0 | 114 |
| none | 50 | 396.0 / 415.6 / 427.6 | 86.7 / 96.6 / 117.2 | 1.0 / 1.1 / 4.4 | 93.1 / 99.2 / 107.3 | 180.4 / 195.0 / 212.2 | 181.0 / 195.0 / 213.0 | 113 |

### python313, 1024 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 317.3 / 342.6 / 356.8 | 795.7 / 826.3 / 925.0 | 81.3 / 86.4 / 89.5 | 0.4 / 0.5 / 0.5 | – | 82.4 / 87.4 / 90.6 | 402.0 / 418.0 / 448.0 | 67 |
| distro | 10 | 1086.1 / 1196.3 / 1313.2 | 1639.4 / 1788.4 / 1844.0 | 90.1 / 96.4 / 97.9 | 1.0 / 1.1 / 1.1 | 82.8 / 86.8 / 89.7 | 178.3 / 182.4 / 184.0 | 1271.0 / 1379.0 / 1495.0 | 111 |
| ext | 10 | 1097.7 / 1425.2 / 1432.7 | 1676.1 / 1998.0 / 2033.0 | 91.3 / 97.6 / 100.3 | 1.0 / 1.1 / 1.2 | 86.7 / 98.4 / 111.7 | 184.6 / 194.7 / 204.7 | 1283.0 / 1620.0 / 1622.0 | 111 |
| none | 10 | 1041.8 / 1147.7 / 1371.4 | 1606.8 / 1923.6 / 2052.9 | 91.0 / 96.3 / 99.4 | 0.9 / 1.0 / 1.0 | 81.9 / 88.1 / 88.5 | 174.5 / 183.4 / 189.4 | 1232.0 / 1321.0 / 1542.0 | 111 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 354.0 / 370.0 / 377.9 | 78.8 / 89.9 / 95.1 | 0.4 / 0.5 / 0.5 | – | 79.6 / 90.7 / 95.9 | 80.0 / 91.0 / 96.0 | 67 |
| distro | 50 | 347.4 / 359.8 / 388.6 | 80.5 / 90.5 / 107.0 | 0.9 / 1.1 / 1.3 | 90.1 / 94.5 / 102.7 | 172.8 / 184.3 / 196.5 | 173.0 / 185.0 / 197.0 | 115 |
| ext | 50 | 394.9 / 410.2 / 427.7 | 82.8 / 90.6 / 99.1 | 0.9 / 1.0 / 1.1 | 91.0 / 97.9 / 101.7 | 175.4 / 186.2 / 193.7 | 176.0 / 187.0 / 194.0 | 112 |
| none | 50 | 393.9 / 411.4 / 435.8 | 80.2 / 92.0 / 97.8 | 0.9 / 1.0 / 1.4 | 91.2 / 98.4 / 104.8 | 174.3 / 185.7 / 198.3 | 175.0 / 186.0 / 199.0 | 113 |

### java21, 128 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 648.3 / 700.0 / 714.0 | 9094.7 / 9659.6 / 9836.2 | 8017.8 / 8520.8 / 8727.0 | 18.8 / 39.2 / 39.3 | – | 8046.6 / 8522.7 / 8746.4 | 8677.0 / 9223.0 / 9461.0 | 109 |
| distro | 0 | – | – | – | – | – | – | – | – |
| ext | 0 | – | – | – | – | – | – | – | – |
| none | 0 | – | – | – | – | – | – | – | – |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 520.2 / 681.1 / 821.1 | 250.5 / 418.8 / 545.0 | 18.8 / 19.5 / 38.2 | – | 265.3 / 425.4 / 545.7 | 266.0 / 426.0 / 546.0 | 112 |
| distro | 0 | – | – | – | – | – | – | – |
| ext | 0 | – | – | – | – | – | – | – |
| none | 0 | – | – | – | – | – | – | – |

### java21, 512 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 642.7 / 728.7 / 857.4 | 2978.6 / 3333.9 / 3485.3 | 1946.3 / 2187.1 / 2233.3 | 1.0 / 18.9 / 19.2 | – | 1962.2 / 2206.5 / 2249.4 | 2598.0 / 2936.0 / 3107.0 | 139 |
| distro | 10 | 4541.4 / 5017.0 / 6282.9 | 9299.4 / 10161.7 / 11806.2 | 4096.4 / 4453.7 / 4812.3 | 18.1 / 21.0 / 21.1 | 86.3 / 91.9 / 94.4 | 4204.3 / 4559.0 / 4912.3 | 8709.0 / 9576.0 / 11196.0 | 262 |
| ext | 10 | 4768.3 / 5712.8 / 5730.8 | 9809.1 / 10826.3 / 11105.4 | 4276.8 / 4597.9 / 4664.4 | 1.7 / 18.5 / 24.2 | 93.9 / 109.7 / 112.1 | 4370.4 / 4710.2 / 4774.1 | 9245.0 / 10084.0 / 10505.0 | 259 |
| none | 10 | 4668.6 / 6105.9 / 6378.4 | 9502.8 / 11584.6 / 11637.2 | 4140.5 / 4605.6 / 4885.6 | 17.9 / 20.9 / 21.1 | 87.8 / 93.6 / 95.0 | 4244.6 / 4710.1 / 4994.6 | 8915.0 / 11055.0 / 11065.0 | 259 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 353.1 / 369.4 / 387.5 | 78.8 / 97.2 / 115.1 | 0.2 / 0.3 / 3.9 | – | 79.3 / 97.7 / 115.6 | 80.0 / 98.0 / 116.0 | 145 |
| distro | 50 | 437.0 / 484.9 / 497.1 | 121.0 / 178.1 / 198.4 | 0.8 / 17.8 / 18.6 | 91.2 / 96.6 / 104.2 | 218.2 / 271.1 / 296.1 | 219.0 / 272.0 / 297.0 | 265 |
| ext | 50 | 381.8 / 439.6 / 485.9 | 117.7 / 169.0 / 190.5 | 0.7 / 17.5 / 19.0 | 91.7 / 97.8 / 101.2 | 218.0 / 262.4 / 289.4 | 218.0 / 263.0 / 290.0 | 261 |
| none | 50 | 385.6 / 420.6 / 458.8 | 117.7 / 153.0 / 183.0 | 0.7 / 15.0 / 19.0 | 94.9 / 101.2 / 109.7 | 213.8 / 256.1 / 281.3 | 214.0 / 257.0 / 282.0 | 264 |

### java21, 1024 MB (p50 / p90 / p99, ms)

| cold | n | init | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 10 | 642.1 / 708.1 / 875.9 | 2009.9 / 2207.9 / 2335.2 | 873.0 / 972.0 / 1062.2 | 0.8 / 0.9 / 13.0 | – | 874.5 / 985.7 / 1063.9 | 1510.0 / 1694.0 / 1940.0 | 140 |
| distro | 10 | 4515.0 / 4970.7 / 5055.9 | 7196.6 / 7831.1 / 8055.4 | 1998.1 / 2146.3 / 2160.4 | 1.6 / 18.6 / 19.0 | 85.7 / 88.6 / 90.6 | 2081.6 / 2247.4 / 2255.4 | 6641.0 / 7219.0 / 7312.0 | 261 |
| ext | 10 | 4536.5 / 5252.9 / 5452.7 | 7124.1 / 8067.8 / 8676.5 | 1952.8 / 2147.6 / 2502.5 | 1.8 / 18.6 / 18.8 | 90.5 / 96.1 / 97.4 | 2050.3 / 2236.1 / 2599.5 | 6559.0 / 7490.0 / 8053.0 | 262 |
| none | 10 | 4683.5 / 5076.0 / 5587.4 | 7373.1 / 7970.5 / 8499.6 | 1995.5 / 2176.5 / 2272.3 | 1.4 / 18.8 / 19.4 | 85.5 / 93.5 / 105.8 | 2078.2 / 2265.6 / 2359.9 | 6675.0 / 7436.0 / 7763.0 | 260 |

| warm | n | client | responseLatency | runtimeOverhead | extensionOverhead | duration | billed | max mem MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 50 | 351.9 / 364.4 / 374.9 | 81.8 / 85.7 / 92.1 | 0.2 / 0.3 / 1.1 | – | 82.4 / 86.2 / 92.6 | 83.0 / 87.0 / 93.0 | 143 |
| distro | 50 | 397.8 / 420.1 / 476.6 | 83.0 / 98.7 / 105.2 | 0.7 / 0.9 / 14.5 | 91.1 / 98.3 / 117.3 | 177.8 / 193.1 / 201.8 | 178.0 / 194.0 / 202.0 | 264 |
| ext | 50 | 356.3 / 370.2 / 386.9 | 81.4 / 95.4 / 111.3 | 0.6 / 0.8 / 12.7 | 95.5 / 103.7 / 114.7 | 179.7 / 195.7 / 209.8 | 180.0 / 196.0 / 210.0 | 268 |
| none | 50 | 350.2 / 369.1 / 390.9 | 83.3 / 92.2 / 102.6 | 0.7 / 0.8 / 7.4 | 94.0 / 99.8 / 102.5 | 176.8 / 189.6 / 199.0 | 177.0 / 190.0 / 200.0 | 264 |

## Reproduce

See "Masking benchmark" in [`benchmarks/README.md`](../../benchmarks/README.md). The layers were
built for arm64 only: `cross` builds of `x86_64` under emulation on an Apple Silicon laptop take
over 30 minutes, so the arm64 binary was built natively in `rust:1.92-alpine` and staged as
`build/dash0_aarch64` before `make build/layer-dash0-<rt>.zip`. The deltas and intervals above
come from the results JSON (bootstrap of the median difference, 4,000 resamples).

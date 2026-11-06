# Extension Performance Benchmarks

Measures the init duration and memory overhead added by the Dash0 Lambda extension across all supported runtimes.

## What it measures

For each runtime version (Python 3.10-3.14, Node.js 20/22/24, Java 17/21/25), two Lambda functions are deployed:
- **Baseline**: minimal handler, no layer
- **Instrumented**: same handler with the Dash0 extension layer

Each function is invoked multiple times with forced cold starts. Init duration and max memory used are extracted from the Lambda REPORT line (via `LogType: Tail`).

## Running via GitHub Actions

Trigger the `Benchmark` workflow manually from the Actions tab. Inputs:
- `cold_start_invocations`: number of cold starts per function (default: 10)
- `commit_results`: whether to commit results back to the repo

## Running locally

```bash
# Prerequisites: AWS credentials configured, layers already published

# Deploy the benchmark stack
cd benchmarks/iac
npm ci
npx cdk deploy

# Install script dependencies & run
cd ../scripts
npm ci
npx tsx run-benchmark.ts

# Clean up
cd ../iac
npx cdk destroy --force
```

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `RESOURCE_PREFIX` | `""` | Prefix for function/layer names |
| `AWS_REGION` | `us-west-2` | AWS region |
| `COLD_START_INVOCATIONS` | `10` | Number of cold starts per function |
| `BENCHMARK_CONCURRENCY` | `10` | Max functions benchmarked in parallel |
| `OUTPUT_FILE` | `results/YYYY-MM-DD.json` | Output file path |

## Results

Results are saved as JSON in `results/`. The script also prints a comparison table to stdout showing init duration and memory overhead per runtime.

## Local signal-export benchmark

This separate benchmark exercises the Rust exporter against an in-process HTTPS receiver. It needs no AWS account or Dash0 credentials. The receiver delays each response independently to model network round trips. The exporter uses its normal protobuf encoding, gzip compression, shared HTTP client, and flush entry point.

Run from the repository root:

```bash
cargo test --release benchmark_signal_exports -- --ignored --nocapture
```

| Variable | Default | Description |
|---|---|---|
| `DASH0_BENCH_DELAY_MS` | `55` | Receiver delay per request; `0` measures local processing without an artificial wait |
| `DASH0_BENCH_ITERATIONS` | `30` | Measured flushes, after five warmup flushes |
| `DASH0_BENCH_RECORDS` | `1` | Records per stream per flush; use `1000` for a larger-batch comparison |
| `DASH0_BENCH_SDK_LOGS` | Unset | Set to also export SDK logs, adding a fourth request alongside traces, Lambda telemetry logs, and metrics |

The benchmark asserts the number of requests received. Keep each stream below the 4 MiB request limit when changing the record count, since the request-count assertion assumes one request per populated stream. Set `DASH0_COMPRESSION=none` to measure without gzip.

For profiles, build once and run the test executable directly. The child flag skips the subprocess isolation used by normal test runs, so Valgrind profiles the exporter and receiver rather than their parent process:

```bash
cargo test --release benchmark_signal_exports --no-run --message-format=json > /tmp/export-build.json
test_binary=$(jq -r 'select(.reason == "compiler-artifact" and .profile.test and .executable != null) | .executable' /tmp/export-build.json)

env -u DASH0_TOKEN_SECRET_ARN DASH0_EXPORT_TEST_CHILD=1 \
  DASH0_BENCH_DELAY_MS=0 DASH0_BENCH_ITERATIONS=200 \
  valgrind --tool=callgrind --callgrind-out-file=/tmp/export.callgrind \
  "$test_binary" --exact otlp::exporter::concurrency_tests::benchmark_signal_exports --ignored --nocapture

env -u DASH0_TOKEN_SECRET_ARN DASH0_EXPORT_TEST_CHILD=1 \
  DASH0_BENCH_DELAY_MS=0 DASH0_BENCH_ITERATIONS=10 \
  DASH0_BENCH_RECORDS=1000 DASH0_BENCH_SDK_LOGS=1 \
  valgrind --tool=massif --massif-out-file=/tmp/export.massif \
  "$test_binary" --exact otlp::exporter::concurrency_tests::benchmark_signal_exports --ignored --nocapture
```

### Parallel-export comparison, 2026-11-06

Linux x86_64 orb, Rust 1.99.1, optimized release builds, gzip enabled. The sequential baseline used commit `601e095465b6d83d2261f652019a143a6aa7d74c` with the same receiver and fixture added. Each timing run measured 40 flushes after five warmups, with one small record per stream. These are local flush times, not deployed Lambda billed durations or regional endpoint measurements.

| Receiver delay | Requests per flush | Sequential median / p95 | Parallel median / p95 |
|---|---:|---:|---:|
| 0 ms | 3 | 0.417 / 0.537 ms | 0.254 / 0.370 ms |
| 0 ms | 4, including SDK logs | 0.533 / 0.614 ms | 0.399 / 0.489 ms |
| 55 ms | 3 | 169.522 / 170.016 ms | 56.816 / 56.971 ms |
| 55 ms | 4, including SDK logs | 225.798 / 227.434 ms | 57.014 / 57.304 ms |

A repeat of the three-request delayed run measured 169.411 ms versus 56.742 ms median. Sub-millisecond zero-delay timings varied between runs; the network-delay reduction is the main result.

Callgrind recorded 638,013,947 versus 645,733,552 instructions for 200 measured three-stream flushes plus warmups and setup, an increase of about 1.2%. Both profiles were dominated by memory operations and gzip encode/decode work. This measures the whole test process, including the receiver and TLS setup, not isolated extension CPU time.

Massif peak heap including allocator overhead, with all four streams and ten measured flushes plus warmups:

| Records per stream | Sequential | Parallel |
|---|---:|---:|
| 1 | 550,984 bytes | 926,696 bytes |
| 1000 | 3,199,112 bytes | 3,566,600 bytes |

The tested concurrency cost was about 0.35 MiB of extra peak heap. On the 1000-record workload, an unprofiled 200-flush run measured 3.226 ms versus 2.205 ms median, with user + system CPU of 0.87 s versus 0.64 s. Receiver allocations are included in all memory measurements. Larger or less compressible payloads can have a different memory cost because up to four requests are now in flight instead of one; each stream's chunks remain sequential.

### High-volume export hardening

The small-fixture memory result above is not an upper bound. A separate local function driver exercised real release extension processes with mocked Lambda APIs and an external HTTPS receiver. Each invocation emitted either 20,000 spans, 20,000 SDK logs and 20,000 function logs with 256-character payloads, or 256 records per stream with 64 KiB payloads. Each workload also sent one metric point. Decoded export totals were approximately 21.8 and 48.1 MiB per invocation.

The original parallel implementation's extension-only Massif peak heap, including allocator overhead, increased from 52.8 MiB sequential to 69.4 MiB for one cold large-payload invocation. Releasing the uncompressed body before awaiting I/O and sharing immutable request bytes between attempts reduced the parallel peak to 61.1 MiB. This still costs about 8.3 MiB more than sequential. These profiles exclude the receiver, unlike the earlier small-fixture profiles.

The hardened exporter was also measured natively with gzip, two Tokio workers, 55 ms added receiver delay, two warmups and eight measured invocations. Tail time starts at handler return and ends when the extension polls for its next lifecycle event. The table reports sample p50 using the upper middle of eight measurements, not AWS billed duration.

| Workload | Early-flush threshold | Sequential tail | Hardened parallel tail | Sequential peak RSS | Parallel peak RSS |
|---|---|---:|---:|---:|---:|
| 20,000 records per stream | Default 2 MiB | 938 ms | 516 ms | 105.8 MiB | 110.3 MiB |
| 256 large records per stream | Default 2 MiB | 2,606 ms | 1,917 ms | 112.6 MiB | 117.2 MiB |

These cases retain a 256 MiB buffer cap and disable periodic flushing to isolate export behavior. They use the default early-flush threshold, not an all-default configuration. Against the previous parallel implementation with invocation-end-only flushing, the large workload measured 1,274 versus 1,312 ms tail p50 and 98.4 versus 88.9 MiB peak RSS. One pair of eight-sample runs cannot establish a precise latency or CPU difference.

A smaller follow-up used the actual default buffer cap for a 128 MiB function configuration, the default 2 MiB threshold and 10 s interval, one Tokio worker, and pinned the function driver, receiver and extension to one CPU. This configures the buffer policy for 128 MiB; it does not enforce a shared 128 MiB memory limit or reproduce Lambda's CPU quota.

| Workload per stream | Sequential tail | Parallel tail | Sequential extension peak RSS | Parallel extension peak RSS |
|---|---:|---:|---:|---:|
| 4,000 records, 256-character payloads | 354 ms | 175 ms | 24.7 MiB | 25.8 MiB |
| 32 records, 64 KiB payloads | 379 ms | 235 ms | 21.1 MiB | 19.2 MiB |

All runs checked exact record IDs for every span and both log streams, the metric value, and absence of export/buffer-full errors. All passed. Native timing, CPU and RSS measurements exclude profiler instrumentation. Real low-memory Lambda tests with handler memory included, cold/warm TLS, freeze/resume and shutdown deadlines remain release gates; local results do not establish those properties.

The CI Rust suite includes HTTPS-receiver tests for concurrent export, sequential chunks, newly arriving data, empty/single signals, exclusions, handler pinning, independent timeouts and exhausted retries, all-stream 429/503/timeouts and recovery, runtime-error correlation/completion, and the mocked Extensions API event loop in both send modes and spindown:

```bash
cargo test otlp::exporter::concurrency_tests
```

### AWS staging validation, 2026-11-06

Disposable manual-instrumentation layers compared the same sequential baseline with the hardened parallel exporter on Python 3.12 Lambda functions in `eu-central-1`. Both x86_64 and arm64 were tested at 128 and 256 MB. Every handler retained and touched 48 MiB of heap. Gzip, the default buffer cap, the 2 MiB early-flush threshold and the 10-second interval remained enabled. Synthetic telemetry went to the real development Dash0 ingress, not a local receiver.

The many-record fixture emitted 4,000 spans and 4,000 SDK logs with distinct 256-character payloads, 512 function logs, and one metric point. The large-record fixture emitted 64 spans and 64 SDK logs with 32 KiB payloads, 16 function logs with 16 KiB payloads, and one metric point. Fixtures were pre-encoded outside the timed handler and submitted in multiple incoming requests. These test the manual layer and actual Runtime/Extensions/Telemetry APIs, not the language distros.

Five warm samples per cell, with alternating variant order, produced these median CloudWatch REPORT durations. REPORT duration includes handler and extension work; it is not just export tail time. Cold pilots and profiling are excluded.

| Architecture | Memory | Many, sequential / parallel | Large, sequential / parallel |
|---|---:|---:|---:|
| x86_64 | 128 MB | 3,672 / 3,738 ms | 3,140 / 3,059 ms |
| x86_64 | 256 MB | 2,118 / 1,908 ms | 1,641 / 1,574 ms |
| arm64 | 128 MB | 2,349 / 2,898 ms | 2,026 / 2,443 ms |
| arm64 | 256 MB | 1,167 / 1,493 ms | 1,000 / 1,275 ms |

The initial arm64 comparison looked slower with parallel exports. Swapping the layers on the two 256 MB arm64 functions reversed the pairwise result:

| Function cohort | Original many / large | After layer swap, many / large |
|---|---:|---:|
| Originally sequential function | Sequential 1,167 / 1,000 ms | Parallel 1,192 / 1,016 ms |
| Originally parallel function | Parallel 1,493 / 1,275 ms | Sequential 1,485 / 1,252 ms |

The faster measurements followed the function cohort across the swap. This demonstrates confounding, not exporter equivalence or a universal speedup. Five samples per cell and one crossover pair do not isolate AWS execution-environment effects. Lower post-handler time can also coexist with higher total duration if export CPU work moves into the handler interval.

All 80 initial warm invocations and 20 crossover warm invocations completed without unexpected handler errors. The 128 MB parallel functions reported maximum memory of 114 MB on x86_64 and 115 MB on arm64, versus 112–114 MB sequential. These are shared execution-environment high-water marks, not isolated exporter allocations. No OOM occurred within this workload envelope; approximately 13–14 MB of reported headroom is not a general memory-safety guarantee.

Exact-ID checks against Dash0 passed for 130 loaded invocations, including cold pilots, the crossover, and lifecycle cases: 219,304 spans, 219,304 SDK logs, 28,552 function logs and 130 expected metric values. Counts, distinct record IDs and range boundaries detected omissions and duplicates. Twenty lifecycle cases additionally checked trace/handler/log correlation using context independently recorded by the function. Expected handler errors and timeouts were checked for error status and successful delivery, not counted as unexpected failures. Empty drain helpers and deliberately failing receiver calls are separate cohorts, not extra delivery passes.

Both architectures at 128 MB exercised handler errors, five-second timeouts and recovery, next-invocation flushing, and idle/resume. All four functions reused the same boot identifier and log stream after idle gaps of 79–183 seconds. This establishes environment reuse and eventual delivery, not survival of a particular TLS connection or direct observation of the freeze transition.

A separate loopback HTTPS receiver inside the 256 MB x86_64 functions tested delayed responses, 429, 503, request timeouts and recovery. All 14 final cases passed per-stream attempt counts and retry-body hash checks. With a 250 ms response delay, the receiver observed four overlapping requests for parallel versus one for sequential. One cold delayed trial measured 1,106 versus 412 ms after the handler, and total REPORT duration of 1,516 versus 826 ms. This is controlled fault/overlap evidence, not a regional latency benchmark. Timed-out requests can remain active server-side: the parallel timeout case reached eight receiver handlers despite only four client-side export streams.

An initial receiver certificate incorrectly had `CA:true`; the TLS client rejected it before requests arrived. Regenerating the test end-entity certificate with `CA:false` fixed that fixture failure. Those failed setup calls are excluded from the 14 corrected cases.

Configuration rollover did not produce a matching Extensions API `SHUTDOWN` event or drain the deliberately buffered fixture within the initial 35-second observation window. During cleanup, two actual `spindown` events were captured for earlier parallel 256 MB arm64 environments. One log stream matched an initially buffered pilot, whose four spans, four SDK logs, four function logs and handler were subsequently verified in Dash0. This is limited real-shutdown delivery evidence. No timestamped export completion was retained to prove the shutdown deadline, and low-memory shutdown under backpressure was not exercised. Those remain open release checks; the targeted staging run also does not replace the repository's deployed language-distro integration suites.

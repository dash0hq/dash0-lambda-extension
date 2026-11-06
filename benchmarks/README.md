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

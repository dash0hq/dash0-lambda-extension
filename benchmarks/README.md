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

## Masking benchmark (distro vs extension)

`iac/lib/masking-benchmark-stack.ts` and `scripts/run-masking-benchmark.ts` compare where secret masking runs: in the language distro, or in the extension (`DASH0_MASK_IN_EXTENSION=true`). Node.js 22, Python 3.13 and Java 21, arm64, at 128, 512 and 1024 MB. Each function makes five HTTP POSTs with ~4 KB JSON bodies to a sink Function URL in the same stack, which also receives the extension's OTLP export and logs how many secrets reach it. Results and method: [`docs/design/central-masking-aws-benchmark.md`](../docs/design/central-masking-aws-benchmark.md).

```bash
export RESOURCE_PREFIX=cmask-bench-
# Publish one layer per runtime (make build/layer-dash0-<rt>.zip, then aws lambda publish-layer-version)
export MASKING_LAYER_NODE=<arn> MASKING_LAYER_PYTHON=<arn> MASKING_LAYER_JAVA=<arn>
cd benchmarks/iac && npx cdk deploy --app "npx ts-node --prefer-ts-exts bin/masking-benchmark.ts"
cd ../scripts && npx tsx run-masking-benchmark.ts           # COLD_START_INVOCATIONS, WARM_INVOCATIONS
npx tsx run-masking-benchmark.ts ../results/<file>.json      # print the tables again
cd ../iac && npx cdk destroy --app "npx ts-node --prefer-ts-exts bin/masking-benchmark.ts" --force
```

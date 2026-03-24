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

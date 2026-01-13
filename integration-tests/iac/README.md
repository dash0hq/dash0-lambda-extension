# Dash0 Lambda Extension - Integration Test Infrastructure

CDK stack for comprehensive integration testing of the Dash0 Lambda Extension across multiple runtimes, architectures, and configurations.

## Stack Structure

The stack creates Lambda functions for testing across all combinations of:

### Dimensions

- **Runtimes**: Python 3.10-3.14, Node.js 20-24, Java 17 & 21
- **Architectures**: x86_64, ARM_64
- **Invocation End**: true, false (controls when telemetry is sent)
- **Tracing**: true (auto-instrumentation), false (synthetic traces)
- **Performance**: baseline, optimized (NEW - enables all performance optimizations)
- **Scenarios**: success, timeout, outofmemory, importerror, exception

### Performance Optimization Testing

**NEW**: Each Lambda function is now deployed in two versions:

1. **Baseline** (no suffix): Default configuration
   - Environment: Standard settings only

2. **Optimized** (`-optimized` suffix): All performance optimizations enabled
   - Environment includes: `DASH0_ENABLE_ALL_OPTIMIZATIONS=true`
   - Enables:
     - Arc<String> for zero-copy sharing (10.3x faster for large payloads)
     - Static HTTP client reuse (70-100x faster warm invocations)
     - Async-aware RwLock (better tail latency)
     - Lazy protobuf decode (2-3x throughput)

### Function Naming Convention

Function names follow this pattern to fit AWS Lambda's 64 character limit:
```
{runtime}-{scenario}-{traced}-ie{invEnd}-{arch}-{perf}
```

Where:
- `runtime`: e.g., `nodejs-20-x`, `python3-10`, `java-21`
- `scenario`: `success`, `timeout`, `outofmemory`, `importerror`, `exception`
- `traced`: `t` (instrumented) or `f` (synthetic traces only)
- `ie{invEnd}`: Invocation end - `iet` (true) or `ief` (false)
- `arch`: `x86` or `arm`
- `perf`: `base` (baseline) or `opt` (all optimizations enabled)

### Example Function Names

```
# Baseline version
nodejs-20-x-success-t-iet-x86-base

# Optimized version (same config + all performance optimizations)
nodejs-20-x-success-t-iet-x86-opt
```

## A/B Testing

The dual deployment enables direct A/B comparison:

```bash
# Invoke baseline version
aws lambda invoke --function-name nodejs-20-x-success-t-iet-x86-base out.json

# Invoke optimized version
aws lambda invoke --function-name nodejs-20-x-success-t-iet-x86-opt out.json
```

Compare CloudWatch metrics:
- Duration (execution time)
- Memory usage
- Error rates
- Cold start times

## Useful Commands

* `npm run build`   compile typescript to js
* `npm run watch`   watch for changes and compile
* `npm run test`    perform the jest unit tests
* `npx cdk deploy`  deploy this stack to your default AWS account/region
* `npx cdk diff`    compare deployed stack with current state
* `npx cdk synth`   emits the synthesized CloudFormation template

## Expected Improvements (Optimized vs Baseline)

Based on benchmark results:

| Metric | Expected Improvement |
|--------|---------------------|
| Overall duration | 40-50% faster |
| Large payload operations (100KB) | 10x faster |
| Concurrent workloads | 3-6x faster |
| Warm invocations (connection reuse) | 70-100x faster |
| Tail latency (p95/p99) | Better with async-safe locks |

## Notes

- Total function count: ~2x previous (baseline + optimized for each config)
- All functions share the same IAM role and CloudWatch log group
- Layer ARNs are pre-configured for us-west-2 region
- Functions have 10s timeout and vary in memory (128MB default, 512MB for Java)

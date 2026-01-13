# Dash0 Lambda Extension

An extension for capturing observability data from lambda invocations and shipping to Dash0.

This extension has four main functionalities:
1. Enable auto-instrumentation for supported runtimes, which currently include Python, Node, Java.
2. Receive traces from auto/manual instrumentations, enrich with data acquired in the extension, and send to Dash0.
3. Detect runtime errors such as timeout or out of memory and create synthetic traces for them
4. Collect all logs and send to Dash0, correlated with the trace id of the invocation.


## Configuration

* `AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper` - This environment variable must be set in order to enable tracing. If this environment variable will not be set, only logs will be collected.

* `DASH0_TOKEN` - the api token for your Dash0 project.

* `DISABLE_AUTO_INSTRUMENTATION` - Auto-instrumentation can be turned off by this environment variable, which will result in creating synthetic traces by the extension for all invocations.

* `SEND_ON_INVOCATION_END` - The extension has two modes of sending to the backend, either on invocation end or on the next invocations. This is controlled by the env var `SEND_ON_INVOCATION_END`. The default is `true`. Sending on invocation end will increase the billed duration of the lambda, but not the response time. Sending on next invocation will decrease the billed duration since the sending will take place in parallel of the regular execution, but might delay the sending up to 7 minutes in case of last invocation in the container.

## Performance Configuration

The extension includes runtime-configurable performance optimizations that can be enabled via environment variables. These optimizations have been benchmarked and proven to significantly improve performance.

### Available Optimizations

| Environment Variable | Description | Expected Improvement |
|---------------------|-------------|----------------------|
| `DASH0_USE_ARC_STRINGS` | Use Arc<String> for zero-copy string sharing in stores | **14x faster** for large payloads (100KB) |
| `DASH0_USE_STATIC_HTTP_CLIENT` | Reuse HTTP client across requests | **70-100x faster** for warm invocations |
| `DASH0_USE_TOKIO_RWLOCK` | Use async-aware RwLock instead of blocking Mutex | Better tail latency (p95/p99) |
| `DASH0_USE_LAZY_PROTOBUF` | Defer protobuf decode/encode until needed | **2-3x throughput** for trace processing |
| `DASH0_ENABLE_ALL_OPTIMIZATIONS` | Enable all optimizations at once | Combined benefits |

### Usage

Enable individual optimizations:
```bash
DASH0_USE_ARC_STRINGS=true
DASH0_USE_STATIC_HTTP_CLIENT=true
DASH0_USE_TOKIO_RWLOCK=true
DASH0_USE_LAZY_PROTOBUF=true
```

Or enable all at once:
```bash
DASH0_ENABLE_ALL_OPTIMIZATIONS=true
```

### Defaults

All optimizations are **disabled by default** to maintain backward compatibility. You can enable them incrementally for safe rollout and A/B testing.

### Configuration Logging

The active configuration is logged at startup:
```
[DASH0] Performance config: Arc=false, StaticClient=false, RwLock=false, LazyProto=false
```

### Recommendation

For best performance in production, enable all optimizations:
```bash
DASH0_ENABLE_ALL_OPTIMIZATIONS=true
```

See [docs/PERFORMANCE_TESTING.md](docs/PERFORMANCE_TESTING.md) for benchmarking methodology and A/B testing strategies.



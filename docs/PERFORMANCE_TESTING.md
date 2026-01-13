# Performance Testing Guide

This document describes how to use the benchmarking and profiling infrastructure to measure and verify performance improvements in the Dash0 Lambda extension.

## Table of Contents

- [Quick Start](#quick-start)
- [Understanding the Performance Issues](#understanding-the-performance-issues)
- [Running Benchmarks](#running-benchmarks)
- [Profiling](#profiling)
- [Verification Workflow](#verification-workflow)
- [Understanding Results](#understanding-results)
- [Troubleshooting](#troubleshooting)

## Quick Start

### Prerequisites

```bash
# All required dependencies are in Cargo.toml
# Optional: Install profiling tools
cargo install flamegraph  # For CPU profiling (optional)
```

### 1. Create Baseline (Before Changes)

```bash
make bench-baseline
```

This saves current performance as the "master" baseline for comparison.

### 2. Make Your Changes

Edit code to implement optimizations (e.g., replace String with Arc in store.rs).

### 3. Run Comparison

```bash
make bench-compare
```

View the HTML report:

```bash
open target/criterion/report/index.html
```

### 4. Profile Hotspots (Optional)

```bash
make profile-cpu
open target/profiling/flamegraph-store_cloning.svg
```

## Understanding the Performance Issues

This infrastructure proves and measures four critical performance issues:

### Issue 1: String Cloning Overhead (store.rs)

**Problem**: Every call to `get_event_payload()` clones the entire String payload.

**Location**: `src/store.rs:8-23`

**Current Cost**: ~100-500µs per clone for large payloads

**Proposed Fix**: Use `Arc<String>` for zero-cost sharing

**Expected Improvement**: 10-50x faster for payloads >10KB

**Benchmark**: `make bench-cloning`

### Issue 2: Mutex in Async Context (all stores)

**Problem**: Using `parking_lot::Mutex` blocks async threads.

**Location**: All `Lazy<Mutex<...>>` stores in `src/store.rs`

**Current Cost**: ~10-50µs contention under concurrent load

**Proposed Fix**: Use `tokio::sync::RwLock` or `DashMap`

**Expected Improvement**: 5-10x lower p99 latency for read-heavy workloads

**Benchmark**: `make bench-mutex`

### Issue 3: HTTP Client Recreation (sandbox.rs)

**Problem**: Creating new HTTP client on every request.

**Location**: `src/sandbox.rs:53, 80`

**Current Cost**: ~1-2ms per request (TCP handshake overhead)

**Proposed Fix**: Reuse static client like `HTTPS_CLIENT` in route.rs

**Expected Improvement**: 100x for 100 requests (100-200ms → 2-4ms)

**Benchmark**: `make bench-http`

### Issue 4: Protobuf Decode/Encode Cycles (backend_send.rs)

**Problem**: Unnecessarily decoding and re-encoding protobuf data.

**Location**: `src/backend_send.rs:64-68`, `src/route.rs:282-289`

**Current Cost**: ~200-800µs per cycle

**Proposed Fix**: Lazy evaluation, only decode when modification needed

**Expected Improvement**: 2x throughput by avoiding unnecessary encodes

**Benchmark**: `make bench-protobuf`

## Running Benchmarks

### Run All Benchmarks

```bash
make bench
```

Runs all four benchmark suites. Takes ~5-10 minutes.

### Run Individual Benchmarks

```bash
make bench-cloning    # String cloning vs Arc
make bench-mutex      # Mutex contention
make bench-http       # HTTP client reuse
make bench-protobuf   # Protobuf operations
```

### Create Baseline for Comparison

```bash
make bench-baseline
```

This saves the current performance as a baseline named "master". You should do this:
- Before starting optimizations
- After merging significant changes to master
- When you want to establish a new performance baseline

### Compare Against Baseline

```bash
make bench-compare
```

This runs all benchmarks and compares results against the saved baseline, showing:
- % improvement or regression
- Statistical significance
- Confidence intervals

## Profiling

### CPU Profiling (Flamegraphs)

Generate a flamegraph to identify CPU hotspots:

```bash
make profile-cpu
```

Or profile a specific benchmark:

```bash
./scripts/profile_cpu.sh store_cloning
./scripts/profile_cpu.sh store_mutex
./scripts/profile_cpu.sh http_client
./scripts/profile_cpu.sh protobuf_ops
```

The flamegraph will be saved to `target/profiling/flamegraph-<benchmark>.svg`.

**Reading Flamegraphs**:
- Wide bars = CPU hotspots (spend most time here)
- Click to zoom into specific call stacks
- Look for:
  - `parking_lot::Mutex::lock` - mutex contention
  - `String::clone` / `to_string` - string allocations
  - `prost::Message::decode` + `encode` - protobuf overhead
  - `hyper::Client::new` - HTTP client creation

### Memory Profiling

```bash
make profile-memory
```

This requires valgrind (Linux) or Instruments (macOS).

For DHAT allocation profiling, add to benchmark file:

```rust
use dhat::{Dhat, DhatAlloc};

#[global_allocator]
static ALLOCATOR: DhatAlloc = DhatAlloc;
```

Then run:

```bash
cargo bench --bench store_cloning
# View output in target/dhat/
```

## Verification Workflow

### Workflow 1: Proving String Cloning Overhead

**Step 1: Establish Baseline**

```bash
git checkout master
make bench-baseline
```

**Step 2: Review Current Performance**

```bash
open target/criterion/report/index.html
```

Look for `store_payload_comparison/string_clone/100KB`:
- Expected: ~500µs per operation
- Linear scaling with payload size

**Step 3: Implement Arc Optimization**

Edit `src/store.rs` to use `Arc<String>` instead of `String` in HashMaps.

**Step 4: Measure Improvement**

```bash
make bench-compare
```

**Expected Results**:
- `store_payload_comparison/arc_clone/100KB`: ~10-20µs
- 10-50x improvement shown in criterion report
- Constant time regardless of payload size

**Step 5: Profile to Confirm**

```bash
make profile-cpu
open target/profiling/flamegraph-store_cloning.svg
```

Verify `String::clone` no longer appears as hotspot.

### Workflow 2: Proving HTTP Client Creation Overhead

**Step 1: Baseline**

```bash
make bench-baseline
```

**Step 2: Check Current Performance**

```bash
make bench-http
```

Look for `http_client_multiple_requests/new_per_request/100`:
- Expected: ~150-200ms total (1.5-2ms per request)

**Step 3: Implement Client Reuse**

Edit `src/sandbox.rs:53` and `:80` to use a static client:

```rust
use once_cell::sync::Lazy;
static HTTP_CLIENT: Lazy<Client<HttpConnector>> = Lazy::new(|| Client::new());

// Then use:
let response = HTTP_CLIENT.request(req).await?;
```

**Step 4: Measure Improvement**

```bash
make bench-compare
```

**Expected Results**:
- `http_client_multiple_requests/reuse_client/100`: ~5-10ms
- 15-20x improvement
- Connection reuse eliminates TCP handshake overhead

### Workflow 3: Combined Performance Test

Run all benchmarks and profiling together:

```bash
make perf-test
```

This runs:
1. All benchmarks
2. CPU profiling
3. Opens reports for review

## Understanding Results

### Criterion Output Format

```
store_event_payload_100KB
  time:   [245.32 µs 248.91 µs 252.84 µs]
  change: [-87.234% -86.891% -86.512%] (p = 0.00 < 0.05)
  Performance has improved.
```

**Key Metrics**:
- **time**: `[lower_bound mean upper_bound]` - 95% confidence interval
- **change**: % difference vs baseline (negative = improvement)
- **p-value**: Statistical significance (p < 0.05 = significant)

**What to Look For**:
- **Green/improvement**: Negative % change
- **Red/regression**: Positive % change
- **Statistical significance**: p < 0.05
- **Magnitude**: Larger % = bigger impact

### Throughput vs Latency

- **Throughput**: Operations/second (higher = better)
- **Latency**: Time per operation (lower = better)
- **p50**: Median latency
- **p95/p99**: Tail latencies (more important for Lambda)

### Example Comparisons

**Good Result** (10x improvement):
```
change: [-90.123% -89.891% -89.512%] (p = 0.00 < 0.05)
```

**Slight Regression** (acceptable noise):
```
change: [+2.123% +2.345% +2.567%] (p = 0.23 > 0.05)
```

**Significant Regression** (needs investigation):
```
change: [+45.123% +47.891% +50.512%] (p = 0.00 < 0.05)
```

## Benchmark Details

### store_cloning.rs

**Tests**:
1. `store_payload_comparison` - String vs Arc for 100B, 1KB, 10KB, 100KB
2. `concurrent_string_clone` - 10 threads, 10 operations each
3. `concurrent_arc_clone` - Same workload with Arc
4. `actual_store_operations` - Real `store_event_payload()` / `get_event_payload()` cycles

**Focus Areas**:
- Clone time scaling with payload size
- Concurrent access patterns
- Memory allocation count

### store_mutex.rs

**Tests**:
1. `mutex_contention_parking_lot` - 10, 50, 100 concurrent tasks
2. `mutex_contention_tokio_rwlock` - Same with RwLock
3. `actual_store_under_load` - 50 concurrent reads from store
4. `write_heavy_workload` - Worst case for RwLock

**Focus Areas**:
- Lock acquisition time
- Contention under concurrent load
- Read vs write performance

### http_client.rs

**Tests**:
1. `http_client_new_per_request` - Create client each time
2. `http_client_reuse` - Use static client
3. `http_client_creation_only` - Just client creation overhead
4. `lazy_initialization` - Cold start cost
5. `concurrent_client_usage` - 10 tasks, 10 requests each

**Focus Areas**:
- Client creation time
- Request throughput
- Connection reuse benefits

### protobuf_ops.rs

**Tests**:
1. `protobuf_decode_encode_cycle` - Full cycle with 1, 5, 10, 50 spans
2. `combine_traces` - 1, 10, 50, 100 traces combined
3. `protobuf_payload_sizes` - 1KB, 10KB, 100KB payloads
4. `lazy_vs_eager_decode` - Conditional decoding benefit
5. `json_to_protobuf_conversion` - Fallback path overhead

**Focus Areas**:
- Decode/encode time
- Payload size impact
- Lazy evaluation benefits

## Troubleshooting

### Benchmarks Won't Compile

**Error**: "cannot find `aws_lambda_runtime_api_proxy_rs` in the list of package names"

**Solution**: Make sure `[[bench]]` sections are added to `Cargo.toml`:

```toml
[[bench]]
name = "store_cloning"
harness = false
```

### Flamegraph Not Generating

**Error**: "flamegraph: command not found"

**Solution**: Install flamegraph:

```bash
cargo install flamegraph
```

On Linux, you may also need:

```bash
sudo apt-get install linux-perf  # or linux-tools-generic
```

### Criterion Reports Not Opening

**Issue**: HTML report not generated

**Solution**: Criterion may skip report generation if benchmarks fail. Check for errors:

```bash
cargo bench 2>&1 | grep -i error
```

### Benchmarks Too Slow

**Issue**: Benchmarks take too long

**Solution**: Run individual benchmarks instead of all:

```bash
make bench-cloning  # Just String cloning benchmarks
```

Or reduce sample size (edit benchmark file):

```rust
group.sample_size(10);  // Default is 100
```

### Inconsistent Results

**Issue**: Results vary significantly between runs

**Possible Causes**:
- Other processes running (close browsers, IDEs)
- CPU throttling (disable power saving mode)
- Background tasks (pause Docker, backups)

**Solution**: Run benchmarks in isolation:

```bash
# Close unnecessary apps
make bench-baseline
# Wait for system to settle
sleep 5
make bench-compare
```

## Performance Targets

| Optimization | Target Improvement | How to Measure |
|--------------|-------------------|----------------|
| String → Arc | >10x for 10KB+ payloads | `make bench-cloning` |
| Mutex → RwLock | <50µs p99 under load | `make bench-mutex` |
| HTTP client reuse | >15x for 100 requests | `make bench-http` |
| Lazy protobuf decode | >2x throughput | `make bench-protobuf` |

## Best Practices

1. **Always create baseline before changes**: `make bench-baseline`
2. **Focus on p95/p99 latencies**: Mean can be misleading
3. **Run multiple times**: Verify improvements are consistent
4. **Profile to confirm**: Flamegraphs show what's actually happening
5. **Test realistic workloads**: Match Lambda invocation patterns
6. **Watch for allocation count**: Not just execution time
7. **Keep baseline up to date**: Re-baseline after merging optimizations

## Next Steps

After setting up benchmarking:

1. **Run initial baseline**: `make bench-baseline`
2. **Review current performance**: `open target/criterion/report/index.html`
3. **Identify worst offenders**: `make profile-cpu`
4. **Implement optimizations** (one at a time)
5. **Verify improvements**: `make bench-compare`
6. **Document results** in this file

## Continuous Improvement

As you make optimizations, update this document with:
- Actual measured improvements
- Lessons learned
- New performance targets
- Additional benchmarks needed

---

**Last Updated**: 2026-01-13
**Maintainer**: Performance Engineering Team

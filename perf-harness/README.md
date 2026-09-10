# Local perf harness

Fast, local, no-AWS-needed tooling for iterating on the extension's cold-start
path and binary size. Complements `../benchmarks`, which measures real,
AWS-deployed, per-language-runtime numbers -- this harness is for verifying a
specific code change before it ever needs a deploy.

It runs the real release binary against a minimal mock of the Lambda
Extensions/Runtime API (`mock_lambda_api.py`) and measures:

- **`coldstart`**: wall-clock time from process spawn to `Registered with
  accountId` (parsed from the binary's own JSON tracing logs), plus binary
  size. Pass `--secret-arn` to exercise the Secrets Manager token path
  instead of the plain `DASH0_TOKEN` fast path.
- **`busyloop`**: regression test for the `get_next()` no-backoff bug. Starts
  the extension, lets it register, then kills the mock Runtime API mid-run
  (simulating the socket becoming briefly unreachable) and samples CPU% for
  a few seconds. Unfixed code pegs a core near 100%; fixed code should settle
  back down.

## Usage

```bash
cd perf-harness
./bench.py build
./bench.py coldstart --runs 15 --label baseline --out results/baseline.json
./bench.py coldstart --runs 15 --label baseline-secrets \
    --secret-arn arn:aws:secretsmanager:us-west-2:123456789012:secret:x \
    --out results/baseline-secrets.json
./bench.py busyloop --duration 6 --label baseline --out results/baseline-busyloop.json

# ... make a code change, rebuild ...
./bench.py build
./bench.py coldstart --runs 15 --label candidate --out results/candidate.json

./bench.py compare --baseline results/baseline.json --candidate results/candidate.json
```

## Caveats

- Builds and runs whatever `cargo build --release` produces on the machine
  running it (e.g. native arm64/macOS during development), **not** the
  `aarch64-unknown-linux-musl` binary that actually ships in the Lambda
  layer. Binary-size deltas from a given change transfer directly to the
  shipped binary. Cold-start wall-clock numbers are only meaningful as a
  relative, before/after comparison on the same machine -- treat them as
  "did this change help or hurt," not as a prediction of real Lambda
  cold-start latency.
- The `--secret-arn` path with real AWS credentials will make a real network
  call to `secretsmanager.<region>.amazonaws.com`. With fake credentials
  (the default if `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` aren't already
  set) it still makes a real network round trip but fails fast on auth --
  useful for timing the request path without needing a real secret.
- `results/` holds point-in-time JSON snapshots for `compare` to diff. They
  aren't meant to be a historical time series (that's what `../benchmarks/results`
  is for) -- treat them as scratch, safe to overwrite between runs.

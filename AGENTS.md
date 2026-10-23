# AGENTS.md

## Layout

- `src/`: Rust extension binary. Ships as static musl builds for `x86_64` and `aarch64`.
- `opt/entrypoint`: the extension entrypoint, packaged as `extensions/dash0` in every layer.
- `opt/shared.sh`: sourced by every wrapper (`opt/{node,python,java,manual}/wrapper`). A change here affects all four runtimes.
- `opt/node/distro`, `opt/python/distro`, `opt/java/opentelemetry-java-distro`: the per-language OpenTelemetry distros. `opt/manual` has a wrapper only.
- `integration-tests/`: CDK stacks (`iac/`), Serverless stacks (`sls/v3`, `sls/v4`) and vitest suites (`tests/`). `integration-tests/runtimes.ts` is the canonical list of tested runtime versions.
- `sls-plugin/`: the `@dash0/serverless-dash0` npm package.
- `build/` is gitignored output (binaries, layer zips, the aws-sdk tarball). Don't put source there.

## Commands

Run these as CI does (`.github/workflows/ci.yml`):

- Rust: `cargo fmt --all --check` and `cargo test --all-features`
- Wrapper: `bash opt/test/shared.test.sh`
- Node distro, from the repo root:
  1. `bash opt/node/scripts/build-aws-sdk-tarball.sh` (builds the forked `@opentelemetry/instrumentation-aws-sdk` into `build/opentelemetry-instrumentation-aws-sdk.tgz`, which `opt/node/package.json` depends on; the `preinstall` hook runs it too, but CI calls it explicitly first)
  2. `cd opt/node && npm install && npm test`
  3. Build: `npm run build` (tsc + webpack into `opt/node/dist`)
- Python distro:
  - `python -m venv opt/python/distro/.venv && opt/python/distro/.venv/bin/pip install -r opt/python/distro/requirements-dev.txt`
  - `cd opt/python/distro && .venv/bin/pytest`
- sls-plugin: `cd sls-plugin && npm ci && npm run build && npm test`
- Java distro: CI has no unit-test job. The only CI build is in `security.yml`: `cd opt/java/opentelemetry-java-distro && ./gradlew clean -Pversion=1.0.0-SNAPSHOT assemble -x javadoc`.
- Layers: `make build/layer-dash0-<node|python|java|manual>.zip` builds a zip. It needs `cross` and Docker, and every `make` call also runs `aws sts get-caller-identity`. Bare `make` and `make node|python|java|manual` publish to AWS and push to ECR. Don't run them.
- Integration tests run only in CI, on same-repo PRs, against stacks deployed with the `pr-<number>-` prefix. They can't run locally without AWS credentials and deployed layers.

## Conventions

- Commit messages and PR titles follow Conventional Commits with an optional scope: `fix(node): ...`, `feat(logs): ...`, `test(integration): ...`, `ci: ...`, `chore: ...`.
- Run `cargo fmt --all` before you commit Rust code. CI fails on unformatted code.
- Changes to `opt/shared.sh` need a test in `opt/test/shared.test.sh`, and you must check them against all four wrappers.
- Don't edit `sls-plugin/src/versions.json` or the plugin version by hand. `deploy.yml` syncs both after each release and opens a `chore: sync 'latest' layer versions` PR.
- When you add or drop a runtime version, update `integration-tests/runtimes.ts`. The CDK stacks and the tests both read it.

## Gotchas

- `build-aws-sdk-tarball.sh` exits early if the tarball already exists. After the fork branch changes, delete `build/opentelemetry-instrumentation-aws-sdk.tgz` to rebuild it.
- The wrappers source `/opt/shared.sh` by absolute path, the way it sits in the Lambda layer. To test it locally, use `opt/test/shared.test.sh`, which sources the repo copy.
- Node `npm test` writes `opt/node/test-spans-*.json`, and `pytest` writes `__pycache__/` under `opt/python/distro/src`. Neither is gitignored, so delete them before you commit.

## Secret masking

When you change how payloads are masked, update the customer docs in the same change:

- `docs/masking.md`: the per-option examples and the Node.js / Python / Java markers on each section
- `README.md`, section "Secret Masking": the default patterns and the runtime support table

This covers any of:

- the extension: `src/otlp/masking.rs`, and the payload and env-var paths that call it
- the wrapper mapping: `map_masking_env` in `opt/shared.sh`
- what a distro captures or how it masks it: the instrumentations under `opt/node/distro`, `opt/python/distro` and `opt/java`

Extension-side payload masking (`DASH0_MASK_IN_EXTENSION`, `src/otlp/payload_masking.rs`) runs in the export builders in `src/otlp/exporter.rs`, never in the receivers: the distros' `forceFlush` waits for `/v1/traces` to return, so receive-time work delays the function's response. Its attribute table is tested against `src/otlp/testdata/masking_inventory.json`, generated from the per-distro attribute inventory. When a distro starts capturing a new payload attribute, add its row to `testdata/distro_inventory.json` and an entry with an `expect` to `masking_inventory.json`; the completeness test fails otherwise. Every example in `docs/masking.md` has a `docs_*` test in `src/otlp/payload_masking_docs_tests.rs` (traceability: `docs/design/central-masking-test-matrix.md`); change both together. `tests/masking_e2e.rs` runs the binary against a fake Lambda API and reads the exports through `DASH0_TEST_EXPORT_DIR`, a seam compiled only into debug builds; `cargo bench --bench masking` measures the cost. `tests/no_payload_in_logs.rs` fails when a log statement formats a body or attribute value: log sizes, or add a control-plane message to its allowlist with a reason.

Generate the examples by running the real code, not by hand. Use `opt/node/test/masking-env.test.ts` for Node.js and `opt/test/shared.test.sh` for the wrapper. Keep the default patterns in `src/otlp/masking.rs`, `opt/shared.sh` and the README identical.

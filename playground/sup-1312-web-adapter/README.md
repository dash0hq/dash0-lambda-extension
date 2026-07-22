# SUP-1312 Playground: Dash0 Lambda Extension + AWS Lambda Web Adapter

Reproducible setup for [SUP-1312](https://linear.app/dash0/issue/SUP-1312) —
`AWS_LAMBDA_EXEC_WRAPPER` conflicts between the
[AWS Lambda Web Adapter](https://github.com/awslabs/aws-lambda-web-adapter) (LWA)
and the Dash0 Lambda extension. Starling Bank runs a web app behind the LWA
(`AWS_LAMBDA_EXEC_WRAPPER=/opt/bootstrap`) and also wants Dash0 monitoring
(`AWS_LAMBDA_EXEC_WRAPPER=/opt/wrapper`) — but the wrapper slot only fits one.
Their chained-wrapper workaround then ran into
`Attempted duplicate registration of X` from `@opentelemetry/api`, because their
in-app OTel SDK collides with the layer's auto-instrumentation.

This CDK app deploys one Express app in six configurations (each with a public
Function URL) so every state — both baselines, the conflict, the repro of the
customer's error, and two candidate fixes — can be exercised side by side.

## Why the conflict exists (mechanics)

Two independent collisions:

1. **One wrapper slot.** `AWS_LAMBDA_EXEC_WRAPPER` is a single env var. LWA
   wants `/opt/bootstrap` (it replaces the runtime loop: starts the web app
   defined by the handler script and translates Runtime API events into HTTP
   requests against `PORT`). Dash0 wants `/opt/wrapper` (it points
   `AWS_LAMBDA_RUNTIME_API` at the extension's Runtime API proxy on
   `127.0.0.1:9009`, sets `OTEL_*` env, and injects the Node auto-instrumentation
   via `NODE_OPTIONS="--import /opt/init.mjs"`, then `exec`s its arguments).

2. **One set of OTel globals per process.** If both the Dash0 distro *and* an
   in-app OTel SDK initialize, the second `registerGlobal()` call logs
   `Attempted duplicate registration of ...` and is ignored — whichever SDK
   registers first owns tracing.

### The chained wrapper

Because Dash0's `/opt/wrapper` finishes with `exec "$@"`, the two wrappers
compose cleanly. [`chained-wrapper-layer/dash0-adapter-wrapper`](chained-wrapper-layer/dash0-adapter-wrapper)
takes the wrapper slot itself and does:

```bash
exec /opt/wrapper /opt/bootstrap "$@"
```

Result:

```
Lambda service
  └─ /opt/dash0-adapter-wrapper <runtime cmd>      (this layer)
       └─ /opt/wrapper                             (Dash0: env + runtime-api proxy + NODE_OPTIONS)
            └─ exec /opt/bootstrap                 (LWA: becomes the runtime client)
                 ├─ polls AWS_LAMBDA_RUNTIME_API=127.0.0.1:9009   → Dash0 extension proxy
                 └─ starts handler script run.sh → node server.js (inherits NODE_OPTIONS
                                                                    → Dash0 auto-instrumentation)
meanwhile (independent of any wrapper):
  /opt/extensions/dash0            Dash0 extension: OTLP receiver + log collection on :9009
  /opt/extensions/lambda-adapter   LWA extension registration
```

The extension's port `9009` multiplexes both the Runtime API proxy **and** the
OTLP receivers (`/v1/traces`, `/v1/metrics`, `/v1/logs` — see `src/route.rs`),
which is what the manual-instrumentation path in the main README uses.

## Scenarios

The **Result** column reflects the deployed test run on 2026-07-22
(eu-west-1, dash0-dev, dataset `raphael-web-adapter`, Dash0 node layer v5,
LWA layer v28) — details in [Findings](#findings-tested-2026-07-22).

| # | Function | Wrapper | Layers | In-app SDK | Result |
|---|----------|---------|--------|-----------|-------------|
| 01 | `sup1312-01-adapter-baseline` | `/opt/bootstrap` | LWA | no | ✅ Baseline: web app works |
| 02 | `sup1312-02-dash0-baseline` | `/opt/wrapper` | Dash0 | no | ✅ Baseline: full traces in Dash0 (invocation + handler + client spans) |
| 03 | `sup1312-03-chained` | `/opt/dash0-adapter-wrapper` | Dash0 + LWA + chained | no | ⚠️ App runs; distro loads & exports, but the extension **discards all app spans** (`/v1/traces has no invocation IDs`); only telemetry-derived invocation spans arrive |
| 04 | `sup1312-04-chained-app-sdk` | `/opt/dash0-adapter-wrapper` | Dash0 + LWA + chained | yes | ✅ **Starling repro**: `@opentelemetry/api: Attempted duplicate registration of API: context` in CloudWatch; app keeps serving, in-app SDK is inert |
| 05 | `sup1312-05-app-sdk-via-proxy` | `/opt/dash0-adapter-wrapper` + `DASH0_DISABLE_AUTO_INSTRUMENTATION=true` | Dash0 + LWA + chained | yes → `127.0.0.1:9009` | ❌ No duplicate registration, but all app spans **discarded** by the extension (same root cause as 03) |
| 06 | `sup1312-06-no-dash0-wrapper` | `/opt/bootstrap` | Dash0 + LWA | yes → `127.0.0.1:9009` | ❌ Same as 05: extension OTLP receiver discards everything (42/42 exports) |
| 07 | `sup1312-07-app-sdk-direct` | `/opt/bootstrap` | Dash0 + LWA | yes → Dash0 ingress directly | ✅ **Works**: full app traces in Dash0 (server + express + client spans, every request), plus extension invocation spans/metrics |

All functions run the same bundle: `server.js` (Express; started via `run.sh`
for LWA scenarios), `handler.js` (plain handler for scenario 02), and `otel.js`
(the in-app SDK, enabled with `INIT_APP_OTEL=true` — the stand-in for
Starling's own OTel setup).

## Deploy

Prereqs: Docker (for asset bundling), AWS credentials, CDK bootstrap in the
target account/region.

```bash
cd playground/sup-1312-web-adapter
npm install

export DASH0_TOKEN=...                # or DASH0_DEV_API_TOKEN
# optional overrides:
# export DASH0_ENDPOINT=https://ingress.eu-west-1.aws.dash0.com:4318   # default is dash0-dev
# export DASH0_DATASET=raphael-web-adapter                            # adds Dash0-Dataset routing
# export DASH0_NODE_LAYER_ARN=arn:aws:lambda:eu-west-1:115813213817:layer:dash0-extension-node:5
# export LWA_LAYER_ARN=arn:aws:lambda:eu-west-1:753240598075:layer:LambdaAdapterLayerX86:28
# export RESOURCE_PREFIX=sup1312-     # default
# export DASH0_EXTENSION_LOG_LEVEL=debug

npx cdk deploy
```

The stack outputs one Function URL per scenario.

## Exercise it

```bash
URL=https://<function-url>/
curl "$URL"              # basic request; response echoes wrapper/runtime-api/NODE_OPTIONS state
curl "$URL/downstream"   # outbound HTTPS call -> should produce an HTTP client span
curl "$URL/error"        # 500 response
```

Then check:

- **CloudWatch logs** (single shared log group, see `LogGroupName` output):
  - scenario 04 must show `Attempted duplicate registration of` (the customer's
    exact symptom; it is a diag error, not a crash — the app keeps working but
    the in-app SDK is inert).
  - the `[app]` / `[app-otel]` / extension log lines show which components ran.
- **Dash0**: each scenario reports with `OTEL_SERVICE_NAME=sup1312-<scenario>`.
  Compare spans (server/client spans, invocation spans, payload log records)
  and log correlation across scenarios 02, 03, 05, 06.

## Findings (tested 2026-07-22)

### 1. The Runtime API proxy can never see LWA traffic — by LWA's architecture

LWA's `/opt/bootstrap` is literally:

```bash
#!/bin/bash
exec -- "${LAMBDA_TASK_ROOT}/${_HANDLER}"
```

It only turns the runtime process into the web app. The actual Lambda↔HTTP
translation runs in **LWA's external extension process**
(`/opt/extensions/lambda-adapter`), which Lambda starts with the *original*
environment — `AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001`. Exec wrappers cannot
modify extension processes, so Dash0's `AWS_LAMBDA_RUNTIME_API=127.0.0.1:9009`
rewrite never reaches the process that polls `/runtime/invocation/next`
(verified: `Got invocation next` appears only in scenario 02's logs).
Consequences with LWA, regardless of wrapper chaining:

- No runtime-proxy features: no event/response payload capture, no
  request-scoped enrichment from the proxy.
- `CURRENT_INVOCATION_ID` in the extension is never set.

The chained wrapper *does* correctly deliver everything env-based: `OTEL_*`
setup and `NODE_OPTIONS --import /opt/init.mjs` (the distro demonstrably loads
and instruments inside the web app process).

### 2. The extension discards all OTLP received from an LWA-served app

`src/otlp/receiver.rs` associates incoming `/v1/traces` payloads with an
invocation: span attribute `faas.invocation_id` (only set by the lambda handler
instrumentation, which is inert under LWA — `No handler file was able to
resolved ... /var/task/run`) or fallback `get_current_invocation_id()` (never
set, see finding 1). Neither exists → `"/v1/traces has no invocation IDs,
discarding trace"` → **every app span is dropped** (42/42 exports in scenarios
05/06, 6/6 batches in 03). This kills both the auto-instrumentation path (03)
and the "app SDK → local extension receiver" path (05, 06).

### 3. The duplicate-registration error is reproduced and understood

Scenario 04 logs the customer's exact error
(`Attempted duplicate registration of API: context`). It is a diag error, not
a crash: whichever SDK registers first (the distro, loaded via NODE_OPTIONS
before app code) owns the process globals; the in-app SDK's registration is
ignored. Guidance stands: exactly one SDK per process — set
`DASH0_DISABLE_AUTO_INSTRUMENTATION=true` (or don't chain the wrapper) when the
app has its own SDK.

### 4. What works today: in-app SDK exporting directly to the Dash0 ingress

Scenario 07 (`APP_OTEL_EXPORT_MODE=direct`: OTLP HTTP straight to
`DASH0_ENDPOINT` with `Authorization` + `Dash0-Dataset` headers) delivers
complete app traces — `GET /downstream` server spans, express middleware
spans, outbound client spans — for every request, alongside the extension's
telemetry-derived invocation spans and FaaS metrics. Caveats:

- App traces and the extension's invocation spans have different trace IDs
  (no correlation without propagation into the event).
- Spans export synchronously per request (SimpleSpanProcessor); at higher
  rates a batch processor + response-hook flush would be needed.

### 5. Side observation: extension log shipping reported 200 but no logs queryable

In all scenarios the extension logs `Sent logs (count=N) ... status=200 OK`,
and FaaS **metrics** for the same functions arrive in the dataset — but
`/api/logs` (and `SELECT count(*) FROM logs` over 30 days) returns zero records
in dataset `raphael-web-adapter`. Needs a separate look (ingestion drop? log
pipeline vs dataset header? dev-environment quirk?). Until explained, the
"logs-only mode" recommendation is on shaky ground.

## Proposed fixes

**Product fix (extension), enables scenarios 05/06 and mostly 03:** in
`receiver.rs`, when a trace has no invocation IDs, fall back to
`get_last_seen_invocation_start()` — it is populated from Telemetry API
`platform.start` events (`log_processing.rs`), which *do* flow in LWA
scenarios — instead of discarding. Optionally forward spans un-associated as a
last resort rather than dropping them.

**Customer guidance today (Starling):**

1. Keep `AWS_LAMBDA_EXEC_WRAPPER=/opt/bootstrap` (Web Adapter). Do not chain.
2. Keep their in-app OTel SDK as the only tracer (no duplicate registration).
3. Point its exporter directly at the Dash0 ingress
   (`https://ingress...dash0.com:4318/v1/traces` + `Authorization: Bearer` +
   `Dash0-Dataset` headers) — scenario 07.
4. Keep the Dash0 layer attached for FaaS metrics and invocation spans
   (log delivery pending finding 5).
5. Once the extension fallback ships, they can switch the exporter to
   `http://127.0.0.1:9009` (scenario 06) to get local, non-blocking-ish export
   plus extension enrichment.

## Cleanup

```bash
npx cdk destroy
```

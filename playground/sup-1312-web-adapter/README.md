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

| # | Function | Wrapper | Layers | In-app SDK | Expectation |
|---|----------|---------|--------|-----------|-------------|
| 01 | `sup1312-01-adapter-baseline` | `/opt/bootstrap` | LWA | no | Baseline: web app works, no telemetry |
| 02 | `sup1312-02-dash0-baseline` | `/opt/wrapper` | Dash0 | no | Baseline: extension works as documented (plain handler) |
| 03 | `sup1312-03-chained` | `/opt/dash0-adapter-wrapper` | Dash0 + LWA + chained | no | **Happy path**: web app served by LWA, traced by Dash0 auto-instrumentation |
| 04 | `sup1312-04-chained-app-sdk` | `/opt/dash0-adapter-wrapper` | Dash0 + LWA + chained | yes | **Starling repro**: `Attempted duplicate registration of ...` in logs; app SDK loses, distro owns tracing |
| 05 | `sup1312-05-app-sdk-via-proxy` | `/opt/dash0-adapter-wrapper` + `DASH0_DISABLE_AUTO_INSTRUMENTATION=true` | Dash0 + LWA + chained | yes → `127.0.0.1:9009` | **Fix candidate A**: app SDK is the only tracer, exports through the extension; extension keeps Runtime API proxy (payloads, invocation context, logs) |
| 06 | `sup1312-06-no-dash0-wrapper` | `/opt/bootstrap` | Dash0 + LWA | yes → `127.0.0.1:9009` | **Fix candidate B** ("logs-only" model from the ticket): Dash0 wrapper never runs; extension collects logs via Telemetry API; app SDK exports to the extension's OTLP receiver |

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

## Open questions this playground is meant to answer

1. Does the chained wrapper (03) give full Dash0 functionality — invocation
   spans, payload capture, log/trace correlation — when the LWA is the runtime
   client talking through the Dash0 Runtime API proxy? (The LWA polls
   `/runtime/invocation/next` in a tight loop; per-request the extension sees
   the HTTP event and response as usual.)
2. In 05, do the app-SDK spans and the extension's invocation context line up
   (same trace), or do we get parallel traces? Does
   `DASH0_DISABLE_AUTO_INSTRUMENTATION=true` still produce synthetic spans that
   duplicate the app SDK's server spans (would `DASH0_DISABLE_TELEMETRY_TRACES`
   be the better switch here)?
3. In 06 (no Dash0 wrapper at all), does the extension's OTLP receiver on
   `127.0.0.1:9009` fully work without the Runtime API proxy in the path —
   enrichment, `faas.invocation_id`, correlation with collected logs?
4. Flushing: the web app is long-lived inside the sandbox; spans are exported
   with `SimpleSpanProcessor` immediately, but after the LWA posts the response
   the sandbox may freeze before the exporter's HTTP call completes. Do spans
   arrive reliably, or do we need a flush hook (e.g. LWA's
   `AWS_LWA_PASS_THROUGH_PATH`-style hook or an Express middleware that awaits
   `forceFlush` before responding)?
5. Layer file collisions: none expected (Dash0: `/opt/wrapper`, `/opt/shared.sh`,
   `/opt/init.mjs`, `/opt/extensions/dash0`; LWA: `/opt/bootstrap`,
   `/opt/extensions/lambda-adapter`) — verify at runtime.

## Outcome options for the ticket

- If **03** works end to end: Dash0 can officially document a chained wrapper —
  or ship one in the layer (e.g. `/opt/wrapper-web-adapter`) so customers set a
  single supported value.
- If the customer keeps their own OTel SDK: **05** (extension keeps the runtime
  proxy, `DASH0_DISABLE_AUTO_INSTRUMENTATION=true`) or **06** (logs-only model,
  no Dash0 wrapper) — the deciding factor is which one preserves correlation
  and enrichment, i.e. open questions 2 and 3.
- Either way the guidance for the duplicate-registration error is: exactly one
  SDK may own the process globals — disable Dash0 auto-instrumentation or
  remove the in-app SDK init.

## Cleanup

```bash
npx cdk destroy
```

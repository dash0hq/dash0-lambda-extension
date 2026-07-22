# Adding Dash0 to a Lambda Web Adapter function — the two options

**Context:** SUP-1312 / Starling Bank. A web app runs on AWS Lambda behind the
[AWS Lambda Web Adapter](https://github.com/awslabs/aws-lambda-web-adapter) (LWA).
The team wants Dash0 monitoring on the same function. Out of the box the two
products compete for the single `AWS_LAMBDA_EXEC_WRAPPER` slot, and the
community chained-wrapper workaround produced
`@opentelemetry/api: Attempted duplicate registration` errors.

This document compares the **current state** with the **two supported target
states**, both verified end-to-end in AWS on 2026-07-22 (playground in this
directory, results in the [README](README.md), wiring details in
[proxy-chains.md](proxy-chains.md)).

**Ports used throughout:** `:9001` = real Lambda Runtime API ·
`:9009` = Dash0 extension (Runtime API proxy **and** OTLP receiver on one port) ·
`:8080` = the web app.

---

## Current state

### What runs today (Web Adapter, no Dash0)

The function works, but produces no Dash0 telemetry. The app's own OpenTelemetry
SDK (if initialized) exports wherever it is currently pointed, over the internet,
in the request path.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process"]
      APP["/opt/bootstrap → exec run.sh<br/>web app :8080<br/>+ in-app OTel SDK"]
    end
    LWA["LWA extension process<br/>lambda-adapter"]
  end
  API["Lambda Runtime API :9001"]
  BE[("current OTLP backend<br/>(or nothing)")]
  LWA -- "poll next / post response → :9001" --> API
  LWA -- "event → HTTP → :8080" --> APP
  APP -- "OTLP over the internet" --> BE
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  classDef cloud fill:#eef0f4,stroke:#8a8fa3,color:#23272f;
  class LWA lwa; class APP,API aws; class BE cloud;
```

**Configuration today:**

```
Handler:  run.sh
Layers:   LambdaAdapterLayerX86
Env:      AWS_LAMBDA_EXEC_WRAPPER=/opt/bootstrap
          PORT=8080
          (in-app SDK exporter config, e.g. OTEL_EXPORTER_OTLP_ENDPOINT=…)
```

### What was attempted (chained wrapper) — and why it fails

Chaining `AWS_LAMBDA_EXEC_WRAPPER` (Dash0 wrapper → LWA bootstrap) looks right
but breaks on two independent points, both reproduced in the playground:

1. **The Dash0 proxy is silently bypassed.** Exec wrappers can only modify the
   *runtime process*. LWA's runtime loop runs in its **extension process**
   (`/opt/extensions/lambda-adapter`), which keeps the original
   `AWS_LAMBDA_RUNTIME_API=:9001`. The Dash0 extension never sees an
   invocation, so it discards every span the app sends it
   (`"/v1/traces has no invocation IDs"` — 42/42 exports in testing).
2. **Two SDKs in one process.** The chained wrapper injects Dash0's
   auto-instrumentation via `NODE_OPTIONS`; if the app also initializes its own
   OTel SDK, the second registration is rejected:
   `Attempted duplicate registration of API: context`. The app keeps serving,
   but the in-app SDK is silently inert.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process · env rewritten — but nobody here polls"]
      APP["web app :8080<br/>Dash0 distro + in-app SDK<br/>⚠ duplicate registration"]
    end
    LWA["LWA extension process<br/>env unchanged: :9001"]
    EXT["Dash0 extension :9009<br/>no current invocation"]
  end
  API["Lambda Runtime API :9001"]
  BIN["all app spans discarded"]
  LWA -- "poll → :9001 (proxy bypassed)" --> API
  LWA -- "HTTP → :8080" --> APP
  APP -. "OTLP → :9009" .-> EXT
  EXT -. discard .-> BIN
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  classDef bad fill:#f7e4e2,stroke:#b3423a,color:#5a201b;
  class EXT dash0; class LWA lwa; class APP,API aws; class BIN bad;
```

### The key that unlocks both options

LWA has a first-class setting for exactly this situation:

```
AWS_LWA_LAMBDA_RUNTIME_API_PROXY=127.0.0.1:9009
```

It tells LWA to run its invocation loop **through a runtime-API proxy**. Because
it is a *function-level environment variable*, it reaches LWA's extension
process — which no exec wrapper ever could. With it set, the Dash0 extension
sees every invocation again, and everything downstream (span association,
payload capture, enrichment) works. Both options below rely on it.

---

## Option A — keep the in-app OTel SDK (recommended for Starling)

**For teams that own and want to keep their OpenTelemetry setup.** The app's SDK
stays the only tracer in the process; the Dash0 extension becomes its local,
non-blocking export target and adds invocation spans and FaaS metrics on top.
This was verified as playground scenario 09.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process"]
      APP["web app :8080<br/>in-app OTel SDK (only tracer)"]
    end
    LWA["LWA extension process<br/>AWS_LWA_LAMBDA_RUNTIME_API_PROXY<br/>= 127.0.0.1:9009"]
    EXT["Dash0 extension :9009"]
  end
  API["Lambda Runtime API :9001"]
  D0[("Dash0 ingress")]
  LWA -- "poll next / post response → :9009" --> EXT
  EXT -- "pass-through" --> API
  LWA -- "event → HTTP → :8080 (+ x-amzn-trace-id)" --> APP
  APP -- "OTLP → :9009 (local, no auth needed)" --> EXT
  EXT -- "associate · enrich · export" --> D0
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  classDef cloud fill:#e7efe7,stroke:#2e7d4f,color:#1d3a29;
  class EXT dash0; class LWA lwa; class APP,API aws; class D0 cloud;
```

### Changes, old → new

| | Old | New |
|---|---|---|
| Handler | `run.sh` | unchanged |
| `AWS_LAMBDA_EXEC_WRAPPER` | `/opt/bootstrap` | **unchanged** — no chaining |
| Layers | LWA | LWA + **`dash0-extension-node`** |
| New env vars | — | `AWS_LWA_LAMBDA_RUNTIME_API_PROXY=127.0.0.1:9009`<br/>`DASH0_ENDPOINT=https://ingress.<region>.aws.dash0.com:4318`<br/>`DASH0_TOKEN=…` (or `DASH0_TOKEN_SECRET_ARN`) |
| App code | SDK exports to current backend | exporter URL → **`http://127.0.0.1:9009`**; token config removed from app |
| Remove | any chained-wrapper workaround | — |

Concretely, the app-side diff is one line (plus optionally the propagator):

```js
// before
new OTLPTraceExporter({ url: 'https://<their-backend>/v1/traces', headers: { … } })
// after — token stays in the extension, call never leaves the sandbox
new OTLPTraceExporter({ url: 'http://127.0.0.1:9009/v1/traces' })
```

Optional but recommended: add the X-Ray Lambda propagator
(`@opentelemetry/propagator-aws-xray-lambda`) to the SDK. LWA already forwards
`x-amzn-trace-id` to the app; with the propagator, the app's traces join the
extension's invocation spans in one trace (without it they arrive as separate,
uncorrelated traces — still complete, just not stitched).

### What you get / what to know

- ✅ No duplicate registration — one SDK per process, by construction.
- ✅ App spans + extension invocation spans + FaaS metrics in Dash0
  (verified: 100% of requests, previously 100% discarded).
- ✅ Export is local; the Dash0 token never lives in app code.
- ⚠️ Trace correlation requires the X-Ray propagator (see above).
- ⚠️ Spans that end after the response is posted ride on the next invocation's
  thaw; the last spans before a long idle arrive late. Use a
  `SimpleSpanProcessor` (or flush before responding) rather than a plain batch
  processor.

---

## Option B — remove the in-app SDK, let Dash0 auto-instrument

**For teams that would rather delete their OTel plumbing.** Dash0's
auto-instrumentation loads into the web app via the chained wrapper and traces
express/HTTP/outbound calls automatically. Verified as playground scenario 08 —
including trace correlation out of the box (21/22 traces).

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process · chained wrapper sets NODE_OPTIONS"]
      APP["web app :8080<br/>Dash0 distro auto-instrumentation"]
    end
    LWA["LWA extension process<br/>AWS_LWA_LAMBDA_RUNTIME_API_PROXY<br/>= 127.0.0.1:9009"]
    EXT["Dash0 extension :9009"]
  end
  API["Lambda Runtime API :9001"]
  D0[("Dash0 ingress")]
  LWA -- "poll next / post response → :9009" --> EXT
  EXT -- "pass-through" --> API
  LWA -- "event → HTTP → :8080 (+ x-amzn-trace-id)" --> APP
  APP -- "OTLP → :9009" --> EXT
  EXT -- "associate · enrich · export" --> D0
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  classDef cloud fill:#e7efe7,stroke:#2e7d4f,color:#1d3a29;
  class EXT dash0; class LWA lwa; class APP,API aws; class D0 cloud;
```

The chained wrapper is a one-liner shipped as a tiny layer (or in the function
bundle) — see [`chained-wrapper-layer/dash0-adapter-wrapper`](chained-wrapper-layer/dash0-adapter-wrapper):

```bash
exec /opt/wrapper /opt/bootstrap "$@"
```

Dash0's `/opt/wrapper` sets up the OTel environment and `NODE_OPTIONS`, then
hands off to LWA's `/opt/bootstrap`, which starts the web app with the
instrumented environment.

### Changes, old → new

| | Old | New |
|---|---|---|
| Handler | `run.sh` | unchanged |
| `AWS_LAMBDA_EXEC_WRAPPER` | `/opt/bootstrap` | **`/opt/dash0-adapter-wrapper`** (chained) |
| Layers | LWA | LWA + `dash0-extension-node` + chained-wrapper layer |
| New env vars | — | `AWS_LWA_LAMBDA_RUNTIME_API_PROXY=127.0.0.1:9009`<br/>`DASH0_ENDPOINT=…`<br/>`DASH0_TOKEN=…` |
| App code | in-app OTel SDK init | **deleted** (and its OTel dependencies) |

### What you get / what to know

- ✅ Zero OTel code in the app; instrumentation updates arrive with layer updates.
- ✅ Server + express + outbound-HTTP spans, correlated with the extension's
  invocation spans in the same trace out of the box (the distro ships the
  `xray-lambda` propagator).
- ✅ Payload log records, enrichment, secret masking — the full extension feature set.
- ⚠️ The in-app SDK **must** be removed; keeping it recreates the
  duplicate-registration error (that is Option A's territory).
- ⚠️ The distro batches spans (5s); under sparse traffic the last batch arrives
  on the next invocation. Same freeze/thaw caveat as Option A.
- ⚠️ Minor known gap: the app's server span parents onto the X-Ray segment, so
  the trace tree shows one missing-parent hop (extension polish item).

---

## Side-by-side

| | **Option A** — keep in-app SDK | **Option B** — Dash0 auto-instrumentation |
|---|---|---|
| Playground scenario | 09 | 08 |
| `AWS_LAMBDA_EXEC_WRAPPER` | untouched (`/opt/bootstrap`) | chained (`/opt/dash0-adapter-wrapper`) |
| App code changes | exporter URL (1 line), optional propagator | delete OTel init + dependencies |
| Who owns instrumentation | the team | Dash0 layer |
| Trace correlation | with X-Ray propagator added | built-in (21/22 verified) |
| Custom/manual spans | full control | via `@opentelemetry/api` against the distro's provider |
| Duplicate-registration risk | none (one SDK) | none (SDK removed) |
| Blast radius of migration | smallest | app dependency change |

**Recommendation:** Starling already has a working, tuned OTel setup → start
with **Option A** (four env-ish changes, one-line app diff, nothing removed).
Option B is the destination for teams that want out of the instrumentation
business entirely. Both can be trialed side by side — that is exactly what the
playground deploys.

**Shared open item (Dash0-side):** extension log export currently returns
200 OK but records were not queryable in the test dataset — do not promise log
delivery until that is resolved (see README finding 6). Traces and FaaS metrics
are verified working in both options.

---

## Deep dive: scenario 08 vs 09 mechanics

Both scenarios share the same invocation plumbing (LWA polls through the Dash0
proxy via `AWS_LWA_LAMBDA_RUNTIME_API_PROXY`). They differ in **how the tracer
gets into the app process**, **when spans are exported**, and **what the
resulting trace looks like**.

### Cold start: how the tracer gets into the process

**Scenario 08** — the chained wrapper injects the Dash0 distro before any app
code runs:

```mermaid
flowchart TB
  L["Lambda starts the runtime process<br/>(AWS_LAMBDA_EXEC_WRAPPER)"]
  CW["/opt/dash0-adapter-wrapper<br/>(chained-wrapper layer)"]
  DW["/opt/wrapper — Dash0<br/>sets OTEL_SERVICE_NAME, OTEL_RESOURCE_ATTRIBUTES<br/>OTEL_PROPAGATORS = tracecontext, baggage, xray-lambda<br/>NODE_OPTIONS = --import /opt/init.mjs"]
  LB["exec /opt/bootstrap — LWA<br/>one line: exec run.sh"]
  RS["run.sh → node server.js"]
  N["Dash0 distro loads FIRST via NODE_OPTIONS<br/>registers the global tracer, patches http/express<br/>exporter: OTLP → 127.0.0.1:9009, BatchSpanProcessor"]
  A["app code runs (no OTel code in it)"]
  L --> CW --> DW --> LB --> RS --> N --> A
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  class CW,DW,N dash0; class LB lwa; class L,RS,A aws;
```

**Scenario 09** — no wrapper chaining; the app initializes its own SDK as it
does today:

```mermaid
flowchart TB
  L["Lambda starts the runtime process<br/>(AWS_LAMBDA_EXEC_WRAPPER)"]
  LB["/opt/bootstrap — LWA<br/>one line: exec run.sh"]
  RS["run.sh → node server.js"]
  N["app requires its own otel setup<br/>NodeSDK registers the global tracer<br/>exporter: OTLP → 127.0.0.1:9009, SimpleSpanProcessor<br/>propagators: tracecontext, baggage (default)"]
  A["app code runs"]
  L --> LB --> RS --> N --> A
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef app fill:#e8ecf5,stroke:#5b6ea3,color:#1d2740;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  class LB lwa; class N app; class L,RS,A aws;
```

The one-registration rule is satisfied differently: in 08 the distro registers
first and is the *only* SDK (the app must not init one); in 09 the app SDK
registers first and is the only SDK (no `NODE_OPTIONS` injection happens).

### One warm request, end to end

**Scenario 08** — spans are batched by the distro and typically leave the
process on a *later* thaw:

```mermaid
sequenceDiagram
  participant LWA as LWA extension
  participant EXT as Dash0 ext :9009
  participant API as Runtime API :9001
  participant APP as web app :8080 (distro)
  LWA->>EXT: GET /invocation/next
  EXT->>API: pass-through
  API-->>EXT: event, requestId = A
  Note over EXT: current invocation = A, event payload captured
  EXT-->>LWA: event A
  LWA->>APP: HTTP GET /downstream, header x-amzn-trace-id
  Note over APP: distro extracts x-amzn-trace-id (xray-lambda propagator)<br/>server span + express span + outbound client span<br/>spans buffered in BatchSpanProcessor
  APP-->>LWA: 200 response
  LWA->>EXT: POST /invocation/A/response
  EXT->>API: pass-through
  Note over EXT: invocation span for A built and exported
  Note over APP,EXT: up to 5 s later, usually during invocation B
  APP->>EXT: POST /v1/traces, batch of spans
  Note over EXT: associated (current invocation set), enriched,<br/>exported to the Dash0 ingress
```

**Scenario 09** — `SimpleSpanProcessor` exports each span the moment it ends,
mostly inside the same invocation window:

```mermaid
sequenceDiagram
  participant LWA as LWA extension
  participant EXT as Dash0 ext :9009
  participant API as Runtime API :9001
  participant APP as web app :8080 (app SDK)
  LWA->>EXT: GET /invocation/next
  EXT->>API: pass-through
  API-->>EXT: event, requestId = A
  Note over EXT: current invocation = A
  EXT-->>LWA: event A
  LWA->>APP: HTTP GET /downstream, header x-amzn-trace-id
  Note over APP: default propagators ignore x-amzn-trace-id<br/>server span starts a NEW trace
  APP->>EXT: POST /v1/traces, client span (ends mid-request)
  APP-->>LWA: 200 response
  APP->>EXT: POST /v1/traces, server + express spans (end with response)
  LWA->>EXT: POST /invocation/A/response
  EXT->>API: pass-through
  Note over EXT: invocation span for A built and exported<br/>app spans associated + exported
```

### What lands in Dash0: trace shapes

**Scenario 08 — one correlated trace per request** (verified 21/22). The
distro's `xray-lambda` propagator stitches app spans into the same trace as the
extension's invocation span. Known polish item: the server span's parent is the
X-Ray segment (not exported), so the tree shows one missing hop.

```mermaid
flowchart TB
  subgraph TR8["trace amB1…  — one trace"]
    R8["invocation span — dash0.lambda-extension"]
    O8["aws.lambda.overhead"]
    G8(["X-Ray segment — parent not exported"])
    S8["GET /downstream — server span (distro)"]
    E8["express handler span"]
    C8["GET checkip.amazonaws.com — client span"]
    R8 --> O8
    G8 -.-> S8
    S8 --> E8
    S8 --> C8
  end
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef app fill:#e8ecf5,stroke:#5b6ea3,color:#1d2740;
  classDef gap fill:#f1f2f5,stroke:#8a8fa3,color:#5b6270,stroke-dasharray:4;
  class R8,O8 dash0; class S8,E8,C8 app; class G8 gap;
```

**Scenario 09 — two parallel traces per request** (as deployed, without the
X-Ray propagator in the app SDK). Both are complete; they are just not stitched
together. Adding `@opentelemetry/propagator-aws-xray-lambda` to the app SDK
should merge them into the 08 shape.

```mermaid
flowchart TB
  subgraph TRX["trace amBo…  — extension"]
    R9["invocation span — dash0.lambda-extension"]
    O9["aws.lambda.overhead"]
    R9 --> O9
  end
  subgraph TRY["trace 71c2…  — app SDK"]
    S9["GET /downstream — server span"]
    E9["express handler span"]
    C9["GET checkip.amazonaws.com — client span"]
    S9 --> E9
    S9 --> C9
  end
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef app fill:#e8ecf5,stroke:#5b6ea3,color:#1d2740;
  class R9,O9 dash0; class S9,E9,C9 app;
```

### Mechanics side by side

| | **08 — auto-instrumentation** | **09 — in-app SDK** |
|---|---|---|
| Tracer enters the process via | `NODE_OPTIONS --import /opt/init.mjs` (chained wrapper) | app's own `require('./otel')` |
| Global registration order | distro first, nothing else allowed | app SDK first, nothing else present |
| Span processor | `BatchSpanProcessor` (max 5 s delay, distro default) | app's choice — `SimpleSpanProcessor` tested |
| Export timing | batches, often during the *next* invocation | per span, mostly within the same invocation |
| Export target | `127.0.0.1:9009` (distro default) | `127.0.0.1:9009` (one-line app change) |
| Propagators | `tracecontext, baggage, xray-lambda` (set by wrapper) | SDK defaults (`tracecontext, baggage`) unless extended |
| Trace correlation with invocation span | ✅ built-in (21/22 verified) | ❌ as deployed; ✅ expected with the X-Ray propagator |
| `OTEL_SERVICE_NAME`, resource attrs | set by the Dash0 wrapper | app/function config |
| Instrumentation coverage | distro's full set (http, express, aws-sdk, pg, redis, …) | whatever the app installs |
| Risk if the other tracer sneaks in | app SDK init → duplicate-registration error | chained wrapper added → same error, reversed |
| Spans at risk of arriving late | last batch before a long idle | spans that end after the response is posted |

# SUP-1312 · Proxy chains: Dash0 extension × Lambda Web Adapter

How each side works alone, why chaining `AWS_LAMBDA_EXEC_WRAPPER` silently bypasses
the Dash0 proxy, and the `AWS_LWA_LAMBDA_RUNTIME_API_PROXY` configuration that makes
the combination work. All verified in eu-west-1 (see [README](README.md)).

Ports: **:9001** = real Lambda Runtime API · **:9009** = Dash0 extension (Runtime API
proxy + OTLP receiver on one port).

## Scenario 02 — Dash0 alone: how the extension normally sees everything ✅

The exec wrapper rewrites `AWS_LAMBDA_RUNTIME_API` *inside the runtime process*, so the
runtime polls for events through the extension. Seeing every poll is what powers the
extension: invocation spans, payload capture, and the **current invocation ID** that
incoming OTLP spans get associated with.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process · env rewritten by /opt/wrapper"]
      RT["Node runtime + handler<br/>Dash0 distro via NODE_OPTIONS"]
    end
    EXT["Dash0 extension :9009<br/>Runtime API proxy + OTLP receiver"]
  end
  API["Lambda Runtime API :9001"]
  D0[("Dash0 ingress")]
  RT -- "poll next / post response → :9009" --> EXT
  RT -- "OTLP /v1/traces → :9009" --> EXT
  EXT -- "pass-through" --> API
  EXT -- "associate · enrich · export" --> D0
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  classDef cloud fill:#e7efe7,stroke:#2e7d4f,color:#1d3a29;
  class EXT dash0; class RT,API aws; class D0 cloud;
```

## Scenario 01 — Web Adapter alone: the runtime loop lives in LWA's extension process ✅

LWA's `/opt/bootstrap` is one line — `exec run.sh` — turning the runtime process into
the web app. The Lambda↔HTTP translation happens in `/opt/extensions/lambda-adapter`,
a separate process Lambda starts with the **original** environment. The important
asymmetry: exec wrappers can only change the runtime process's environment — extension
processes are out of reach.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process"]
      APP["/opt/bootstrap → exec run.sh<br/>web app (express) :8080"]
    end
    LWA["LWA extension process<br/>lambda-adapter"]
  end
  API["Lambda Runtime API :9001"]
  LWA -- "poll next / post response → :9001" --> API
  LWA -- "event → HTTP request → :8080" --> APP
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  class LWA lwa; class APP,API aws;
```

## Scenarios 03/04/05/06 — chained wrapper: why the Dash0 proxy is bypassed ❌

The chained wrapper (`exec /opt/wrapper /opt/bootstrap`) delivers everything env-based —
`OTEL_*`, `NODE_OPTIONS` — into the app (instrumentation demonstrably loads). But the
process that actually polls the Runtime API is LWA's extension, whose environment still
says `:9001`. The Dash0 extension never sees a poll, has no current invocation, and
discards every OTLP export from the app: `"/v1/traces has no invocation IDs"`.

Scenario 04 adds the second collision on top: distro + in-app SDK in one process →
`Attempted duplicate registration of API: context`.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process · env rewritten (…=:9009) — but nobody here polls"]
      APP["web app :8080<br/>distro or in-app SDK loaded ✓"]
    end
    LWA["LWA extension process<br/>env unchanged: :9001"]
    EXT["Dash0 extension :9009<br/>never sees a poll →<br/>no current invocation"]
  end
  API["Lambda Runtime API :9001"]
  BIN["discarded:<br/>/v1/traces has no invocation IDs"]
  LWA -- "poll → :9001 (proxy bypassed)" --> API
  LWA -- "HTTP → :8080" --> APP
  APP -. "OTLP → :9009" .-> EXT
  EXT -. "42 / 42 exports" .-> BIN
  classDef dash0 fill:#f3ddb0,stroke:#b07818,color:#3a2f14;
  classDef lwa fill:#c8e6e4,stroke:#14787a,color:#0d3536;
  classDef aws fill:#e2e5ec,stroke:#8a8fa3,color:#23272f;
  classDef bad fill:#f7e4e2,stroke:#b3423a,color:#5a201b;
  class EXT dash0; class LWA lwa; class APP,API aws; class BIN bad;
```

## Scenarios 08/09 — the fix: `AWS_LWA_LAMBDA_RUNTIME_API_PROXY=127.0.0.1:9009` ✅

LWA's own knob for runtime-API-proxy extensions. As a **function-level** env var it
reaches LWA's extension process, routing the invocation loop through the Dash0 proxy —
the one thing the wrapper never could. LWA registers against the real Runtime API and
only routes the invocation loop through the proxy, so there is no registration conflict.

- **Scenario 09 (recommend to Starling):** keep `/opt/bootstrap`, keep their in-app SDK,
  exporter → `http://127.0.0.1:9009`; add the `xray-lambda` propagator for trace
  correlation with the extension's invocation spans.
- **Scenario 08 (full Dash0):** chained wrapper + this knob → auto-instrumented,
  correlated traces (21/22) with zero app changes.

```mermaid
flowchart LR
  subgraph SB["Lambda execution environment"]
    direction LR
    subgraph RP["runtime process"]
      APP["web app :8080<br/>08: Dash0 distro · 09: in-app SDK"]
    end
    LWA["LWA extension process<br/>AWS_LWA_LAMBDA_RUNTIME_API_PROXY<br/>= 127.0.0.1:9009"]
    EXT["Dash0 extension :9009<br/>sees every invocation again"]
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

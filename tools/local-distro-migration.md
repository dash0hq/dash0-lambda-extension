# Migration: External Lumigo Distros to New Local Distros

This document describes the migration of language-specific OpenTelemetry distro packages from external repositories into this repository. The goal is to have a light-weight distro with auto-instrumentation and lambda-specific features such as collecting payloads of http requests.

## Overview

Each Lambda layer bundles a language-specific OpenTelemetry distribution ("distro") that provides auto-instrumentation. Previously, these distros were pulled from external repositories during the build. The migration brings the distro source code directly into this repo under `opt/<language>/distro/`. Additionally, the distro will be light-weighted to include only the necessary components for Lambda auto-instrumentation, and will be customized to collect additional telemetry such as HTTP request payloads.

## Python

### Before

- `opt/python/requirements.txt` contained a single line pointing to an external git repo:
  ```
  git+https://github.com/lumigo-io/opentelemetry-python-distro.git@<commit-hash>
  ```
- `opt/python/Dockerfile` ran `pip install -r requirements.txt` which cloned the repo, built the package, and installed it along with all its transitive dependencies into `/asset-output/python`.

### After

- `opt/python/distro/src/dash0_opentelemetry/` contains the full distro source code.
- `opt/python/distro/requirements.txt` lists the distro's sub-dependencies (opentelemetry packages, instrumentations, etc.).
- `opt/python/Dockerfile` now:
  1. Installs sub-dependencies from `distro/requirements.txt` via pip.
  2. Copies the `dash0_opentelemetry` package source directly into `/asset-output/python/`.
- The package is imported at runtime via `import dash0_opentelemetry` in `otel_wrapper.py`. This works because `/opt/python` is on `PYTHONPATH` and the package directory is placed there.

### Directory structure

```
opt/python/
├── Dockerfile                          # Builds dependencies + copies distro source
├── requirements.txt                    # (legacy, no longer used in build)
├── otel_wrapper.py                     # Lambda wrapper that imports lumigo_opentelemetry
├── wrapper                             # Bash entrypoint script
└── distro/
    ├── requirements.txt                # Sub-dependencies (otel packages, instrumentations)
    └── src/
        └── dash0_opentelemetry/       # Distro source code
            ├── __init__.py             # Entry point, calls init() on import
            ├── VERSION
            ├── dependencies/
            ├── external/
            ├── instrumentations/
            ├── libs/
            ├── processors/
            ├── resources/
            └── utils/
```

### Build flow (Makefile)

```
make python
  → docker build -f opt/python/Dockerfile .
    → pip install -r distro/requirements.txt -t /asset-output/python
    → COPY distro/src/dash0_opentelemetry → /asset-output/python/dash0_opentelemetry
  → docker cp /asset-output/python → build/python
  → assemble ZIP with binaries + wrapper + otel_wrapper.py + build/python
```

## Node.js

### Before

- `opt/node/package.json` declares `@lumigo/opentelemetry` as an npm dependency (pulled from npm registry).
- `npm install` fetches the package and all transitive dependencies into `node_modules/`.
- `webpack` bundles the distro + instrumentation into `dist/init.mjs`.
- Specific `node_modules` that can't be bundled (e.g., `import-in-the-middle`) are copied separately.

### Migration plan

- Copy the `@lumigo/opentelemetry` source into `opt/node/distro/`.  (Only the relevant parts)
- Create `opt/node/distro/package.json` with sub-dependencies.
- Update `opt/node/package.json` to reference the local distro: `"@lumigo/opentelemetry": "file:./distro"`.
- The webpack build and runtime behavior remain unchanged.

### Target directory structure

```
opt/node/
├── package.json                        # References local distro via file:./distro
├── package-lock.json
├── webpack.config.mjs                  # Bundles distro into dist/init.mjs
├── init.mjs                            # ESM entrypoint
├── wrapper                             # Bash entrypoint script
└── distro/
    ├── package.json                    # Sub-dependencies
    └── src/                            # @lumigo/opentelemetry source code
```

## Java

### Before

- No local dependencies at all. The Makefile downloads a pre-built JAR at build time:
  ```
  curl -L -o .../lumigo-opentelemetry.jar https://github.com/lumigo-io/opentelemetry-java-distro/releases/download/v0.19.1/lumigo-opentelemetry-0.19.1.jar
  ```
- The JAR is placed at `/opt/java/lib/lumigo-opentelemetry.jar` and loaded via `-javaagent` in the wrapper script.

### Migration plan

- Copy the Java distro source into `opt/java/distro/`.
- Add a build step (Gradle/Maven) to compile the JAR from local source.
- Update the Makefile to build the JAR locally instead of downloading it.
- The wrapper script and runtime behavior remain unchanged (still loads the same JAR via `-javaagent`).

### Target directory structure

```
opt/java/
├── wrapper                             # Bash entrypoint, sets -javaagent
└── distro/
    ├── build.gradle / pom.xml          # Build configuration
    └── src/                            # Java distro source code
```

## Common patterns

All three languages follow the same migration pattern:

| Aspect | Before | After |
|--------|--------|-------|
| Source location | External repo (git/npm/GitHub releases) | `opt/<lang>/distro/` |
| Dependencies | Resolved transitively during install | Explicitly listed in `distro/requirements.txt` or `distro/package.json` |
| Build | Fetches from network | Fully local (except sub-dependencies from registries) |
| Version control | Pinned by commit hash / version | Source code tracked in this repo |

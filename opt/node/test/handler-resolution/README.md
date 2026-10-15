# Handler file resolution

Notes on the failure mode that `../handler-resolution.test.ts` reproduces, and on the fix
in `opt/node/distro/src/lambdaHandlerResolution.ts`: an instrumented Node function that is healthy in
every observable way and still emits no handler spans.

## The symptom

Three things hold at once, which is what makes this hard to recognise:

- **The function works.** Every invocation returns 200. Nothing throws, nothing retries,
  and the application logs its own requests normally.
- **Invocation metrics still appear in Dash0.** Those are scraped from CloudWatch by the
  AWS integration, so they show up whether or not the extension exports anything.
- **No handler span is ever produced.** Not a failed export, not a dropped batch — the
  span is never created.

The only trace of it is one line in the function's own log group:

```
@opentelemetry/instrumentation-aws-lambda No handler file was able to resolved with one
of the known extensions for the file /var/task/foo/index
```

From the outside this is indistinguishable from a function that was never instrumented.

## Where the bug is not

- **Not the Rust extension.** It never reads the handler string. This is
  instrumentation-only, which is why the reproduction needs no emulator and no AWS
  account.
- **Not the runtime version.** `nodejs24.x` is in the tested matrix.
- **Not a directory prefix in the handler string.** A prefixed handler resolves fine when
  the package follows it; the second test case covers this.
- **Not a stale upstream.** The newest `@opentelemetry/instrumentation-aws-lambda` carries
  identical resolution code, so a version bump does not help.
- **Not a broken deployment package.** The obvious first theory — a bundler flattened its
  output while the handler string kept the source directory — predicts a *broken*
  function. It isn't: the runtime recovers from exactly that, which is the whole point
  below.

## The gap

The Lambda runtime interface client (`dist/function/module-loader.js`, inlined into
`/var/runtime/index.mjs`) resolves the handler module in five steps:

```js
const base = path.resolve(appRoot, moduleRoot, moduleName);

for (const extension of ['', '.js', '.mjs', '.cjs']) {          // steps 1-4
  if (existsSync(base + extension)) return await import(base + extension);
}

return cjsRequire(cjsRequire.resolve(moduleName, {              // step 5
  paths: [appRoot, path.join(appRoot, moduleRoot)],
}));
```

`AwsLambdaInstrumentation.init()` replicates steps 2 to 4 — `statSync` on `.js`, `.mjs`,
`.cjs` — and nothing else. When all three miss, the path keeps its extensionless form, and
**that string is what the require hook is armed on**. The hook is registered for something
the runtime will never load. The handler runs unwrapped, and the patch never fires.
Nothing downstream treats this as an error, because a hook that matches nothing is a
normal state.

Step 5 is where the customer landed, and it is not a corner case. It resolves a **bare
specifier**, so it falls through to Node's global paths — and Lambda puts the task root on
`NODE_PATH`:

```
NODE_PATH=/opt/nodejs/node24/node_modules:/opt/nodejs/node_modules:
          /var/runtime/node_modules:/var/runtime:/var/task
```

So `require.resolve('index')` finds `/var/task/index.js`. **A handler configured as
`foo/index.handler` whose package contains only `index.js` at the root runs perfectly
well: the directory prefix is silently ignored.** Handler prefixes left stale by a bundler
are common, and because nothing breaks, nobody notices until the traces are missing.

This is also why the failure cannot be reproduced by hand outside Lambda. Without
`NODE_PATH`, step 5 returns `MODULE_NOT_FOUND` and the function looks broken — which sends
you looking for a broken deployment package that isn't there.

## Verified against real AWS

Three `nodejs24.x` functions, same deployment package (`index.js` + a sourcemap, nothing
else), differing only in the configured handler:

| `Handler` | package | invocation | warning | spans |
| --- | --- | --- | --- | --- |
| `foo/index.handler` | `index.js` | **200** | **yes** | **0** |
| `index.handler` | `index.js` | 200 | no | 2 |
| `foo/index.handler` | `foo/index/index.js` | `Runtime.Unknown` at init | — | — |

with `DASH0_DEBUG=true` giving, on the first:

```
Instrumenting lambda handler {
  taskRoot: '/var/task',  handlerDef: 'foo/index.handler',
  moduleRoot: 'foo/',     module: 'index',
  filename: '/var/task/foo/index',  functionName: 'handler'
}
HANDLER RAN {"_HANDLER":"foo/index.handler","__filename":"/var/task/index.js"}
```

The third row is worth keeping in mind: on `nodejs24.x` the runtime `import()`s the handler,
and `import()` refuses directory imports. The `ERR_UNSUPPORTED_DIR_IMPORT` escapes the
extension loop, so neither the remaining extensions nor step 5 are tried, and the function
dies at init. Directory modules are not a viable layout on this runtime at all.

## The fix

`opt/node/distro/src/lambdaHandlerResolution.ts` computes what the runtime will actually load — the
same five steps, using `require.resolve` rather than a reimplementation of it, so
`node_modules` precedence, `NODE_PATH` ordering, `exports` maps and symlinks are all
inherited from the same resolver the runtime calls. It expresses the result back as a
handler string and `init.mjs` passes it to upstream's `lambdaHandler` config option, which
replaces `_HANDLER` for resolution purposes.

It returns `undefined` — leaving upstream's behaviour untouched — whenever upstream already
gets it right, and whenever the correction cannot be expressed as a handler string. So the
only deployments whose behaviour changes are the ones that are already silently broken.

Known limitations, both deliberate:

- A handler that resolves **outside the task root** (a module shipped in a layer) is left
  alone; a handler string cannot express that unambiguously.
- When both `<base>` and `<base>.js` exist, the runtime loads the extensionless one and
  upstream picks `.js`. The correction is applied by re-running upstream's own parsing, so
  it cannot override that choice. Exotic, and the function still works — it just isn't
  traced.

The real home for this is upstream: the same logic inside `init()` would fix it for every
OpenTelemetry Lambda user, and this module can be deleted when it lands.

## What the test covers

| `_HANDLER` | Deployment package | Loads? | Span |
| --- | --- | --- | --- |
| `index.handler` | `index.js` | yes | yes |
| `foo/index.handler` | `foo/index.js` | yes | yes |
| `foo/index.handler` | `index.js` + sourcemap | yes, 200 | **none** — the bug |
| &hellip; with the fix applied | `index.js` + sourcemap | yes, 200 | yes |
| `foo/index.handler` | `node_modules/index` package | yes, 200 | yes (with the fix) |
| `foo/missing.handler` | `index.js` | no — `MODULE_NOT_FOUND` | none |
| `foo/index.handler` | `foo/index/index.js` | no — `ERR_UNSUPPORTED_DIR_IMPORT` | none |

Each scenario runs in its own process, for three reasons: `init()` reads the environment
during construction and the require hook it installs is global and caches modules; Node
reads `NODE_PATH` once at startup, so it must be set before the process starts; and **jest
patches `Module._resolveFilename`**, so a resolution assertion made inside jest would be
testing jest's resolver rather than Node's. `runner.mjs` is plain Node, and documents where
it deliberately deviates from the runtime.

## Diagnosing a report like this

```sh
# what the function is actually configured with
aws lambda get-function-configuration --function-name <fn> \
  --query '{pkg:PackageType,handler:Handler,runtime:Runtime}'

# what the deployment package actually contains
aws lambda get-function --function-name <fn> --query Code.Location
# ...then unzip -l the download and compare against the handler path
```

If `Handler` carries a directory prefix that `unzip -l` does not show, this is the bug.

If `PackageType` is `Image`, `Handler` is meaningless — `_HANDLER` comes from the image's
`CMD` or `ImageConfig.Command`, and `LAMBDA_TASK_ROOT` need not be `/var/task`.

`DASH0_DEBUG=true` logs `taskRoot`, `handlerDef`, `moduleRoot`, `module` and `filename`
separately, which the upstream warning collapses into a single joined path.

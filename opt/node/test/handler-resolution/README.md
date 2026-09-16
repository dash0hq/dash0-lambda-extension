# Handler file resolution

Notes on the failure mode that `../handler-resolution.test.ts` reproduces: an
instrumented Node function that is healthy in every observable way and still emits no
handler spans.

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
of the known extensions for the file /var/task/syncjob-service-staging/index
```

From the outside this is indistinguishable from a function that was never instrumented.

## Where the bug is not

- **Not the Rust extension.** It never reads the handler string. This is
  instrumentation-only, which is why the reproduction needs no emulator and no AWS
  account.
- **Not `opt/node/init.mjs`.** It constructs `new AwsLambdaInstrumentation({})` and leaves
  resolution entirely to upstream. The empty config matters — see *Fix options* below.
- **Not the runtime version.** `nodejs24.x` is in the tested matrix. The runtime version
  changes callback support, not path resolution.
- **Not a directory prefix in the handler string.** A prefixed handler resolves fine when
  the package follows it; the second test case covers this.
- **Not a stale upstream.** The newest `@opentelemetry/instrumentation-aws-lambda` carries
  identical resolution code, so a version bump does not help.

## The constraint

The Lambda runtime interface derives the module path from `_HANDLER` using the same
computation the instrumentation does:

```
path.resolve(LAMBDA_TASK_ROOT, dirname(_HANDLER), basename(_HANDLER).split('.')[0])
```

So the runtime and the instrumentation **cannot disagree about the path**. This rules out
the obvious first theory — a bundler that flattens its output while the handler string
keeps the source directory. If the file were genuinely absent, the runtime could not load
it either, and the invocation would fail with `Runtime.ImportModuleError` rather than
return 200. The last test case pins that down as a negative control.

What they disagree about is how that shared path becomes a *file*:

| | turns the path into a file by |
| --- | --- |
| Lambda runtime | Node's resolver — which also resolves a **directory**, through its `index.js` or its package `main` |
| instrumentation | `statSync` on `.js`, `.mjs`, `.cjs` at exactly that path |

When all three stats miss, the path keeps its extensionless form, and **that string is
what the require hook is armed on**. The hook is registered for something the runtime will
never load. Node resolves the directory, the handler runs, and the patch never fires.

Nothing downstream treats this as an error — a hook that matches nothing is a normal
state — so the invocation proceeds exactly as an uninstrumented one would.

## What the test covers

| `_HANDLER` | Deployment package | Loads? | Span |
| --- | --- | --- | --- |
| `index.handler` | `index.js` | yes | yes |
| `syncjob-service-staging/index.handler` | `syncjob-service-staging/index.js` | yes | yes |
| `syncjob-service-staging/index.handler` | `syncjob-service-staging/index/index.js` | yes, 200 | **none** |
| `syncjob-service-staging/index.handler` | `syncjob-service-staging/index/package.json` → `app.js` | yes, 200 | **none** |
| `syncjob-service-staging/index.handler` | `index.js` | no — `MODULE_NOT_FOUND` | none |
| &hellip; plus `lambdaHandler` pointing at the real file | `syncjob-service-staging/index/index.js` | yes | yes |

The two directory shapes reproduce the reported log line *while returning 200*. That
combination is the whole signature; the warning on its own does not distinguish them from
a genuinely broken function.

Each scenario runs in its own process. `init()` reads the environment during construction
and the require hook it installs is global and caches loaded modules, so running them
in-process would leak state between them. `runner.js` also loads the module the way the
runtime does — by the computed path, through Node's resolver — rather than requiring the
bundle file directly. Requiring the file directly is what concealed the constraint above,
because it lets the runtime and the instrumentation appear to disagree about the path.

## Fix options

Neither is applied; this branch is a reproduction.

1. **Pass the handler path through.** Upstream already accepts a `lambdaHandler` config
   value that replaces `_HANDLER` for resolution purposes. `init.mjs` passes `{}`, so
   there is currently no way to correct the path from outside. An env var — say
   `DASH0_LAMBDA_HANDLER` — plumbed into that config unblocks anyone affected, at the cost
   of requiring them to know about it.
2. **Fall back to Node's resolver.** When the three stats miss, try `require.resolve` on
   the base path before giving up. This covers the directory case with no customer-side
   configuration, but diverges from upstream's behaviour.

Worth deciding separately: whether a resolution miss deserves a louder signal than a
`diag.warn`. The warning also prints only the joined path, collapsing `LAMBDA_TASK_ROOT`
and the handler string into a single value — which is why diagnosing this from the log
line alone took several days. `DASH0_DEBUG=true` logs the three values separately.

## Diagnosing a report like this

```sh
# what the function is actually configured with
aws lambda get-function-configuration --function-name <fn> \
  --query '{pkg:PackageType,handler:Handler,runtime:Runtime}'

# what the deployment package actually contains
aws lambda get-function --function-name <fn> --query Code.Location
# ...then unzip -l the download and look at what the handler path resolves to
```

If `PackageType` is `Image`, `Handler` is meaningless — `_HANDLER` comes from the image's
`CMD` or `ImageConfig.Command` instead.

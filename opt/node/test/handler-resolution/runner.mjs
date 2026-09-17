/*
 * Child-process runner for `opt/node/test/handler-resolution.test.ts`.
 *
 * Every scenario needs its own process, for three reasons:
 *
 *   - `AwsLambdaInstrumentation.init()` reads `LAMBDA_TASK_ROOT` and `_HANDLER` during
 *     construction, and the require hook it installs is global and caches modules;
 *   - Node reads `NODE_PATH` once at startup, and `NODE_PATH` is the whole point here
 *     (see below), so it has to be in the environment before the process starts;
 *   - jest patches `Module._resolveFilename`, so a resolution assertion made inside jest
 *     would be testing jest's resolver rather than Node's. This runner is plain Node.
 *
 * What it models
 * --------------
 * The AWS Lambda runtime interface client (`dist/function/module-loader.js`, inlined into
 * /var/runtime/index.mjs) resolves the handler module like this:
 *
 *     const base = path.resolve(appRoot, moduleRoot, moduleName);
 *     for (const extension of ['', '.js', '.mjs', '.cjs']) {
 *       if (existsSync(base + extension)) return await import(base + extension);
 *     }
 *     return cjsRequire(cjsRequire.resolve(moduleName, {
 *       paths: [appRoot, path.join(appRoot, moduleRoot)],
 *     }));
 *
 * `resolveLikeTheRuntime` below is that algorithm. The final step resolves a *bare*
 * specifier, so it falls through to Node's global paths -- and on Lambda `NODE_PATH`
 * contains the task root. That is how a function configured as `foo/index.handler` still
 * loads `/var/task/index.js` when the deployment package has no `foo/` in it at all.
 *
 * Where it deviates, deliberately: the runtime `import()`s the resolved file, this runner
 * `require()`s it. The behaviour under test is *which path the instrumentation arms its
 * hook on*, and require-in-the-middle keeps that observable without dragging ESM loader
 * hooks into the test. The one case where the distinction matters -- a handler path that
 * is a directory, which `import()` rejects outright -- is asserted through the resolution
 * result rather than the load, and is covered in the README.
 *
 * Usage: node runner.mjs '<scenario JSON>'
 *
 *   taskRoot       the deployment package, already on disk (the test builds it, because
 *                  NODE_PATH has to name it before this process starts)
 *   handler        value of `_HANDLER`, i.e. what the function is configured with in AWS
 *   applyFix       when true, pass our corrected handler into the instrumentation config
 *
 * Prints a JSON result on stdout:
 *   { taskRoot, base, loadedFrom, loadError, statusCode, lambdaHandler, spans, diag }
 */

import fs from 'fs';
import path from 'path';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);
const scenario = JSON.parse(process.argv[2]);
const taskRoot = scenario.taskRoot;

process.env.LAMBDA_TASK_ROOT = taskRoot;
process.env._HANDLER = scenario.handler;
process.env.AWS_EXECUTION_ENV = 'AWS_Lambda_nodejs24.x';
process.env.AWS_LAMBDA_FUNCTION_NAME = 'handler-resolution-test';

const { diag, DiagLogLevel } = require('@opentelemetry/api');

// Capture what would reach the function's CloudWatch log group, so the test can assert on
// the exact line customers report.
const diagMessages = [];
const collect = (...args) => diagMessages.push(args.map(String).join(' '));
diag.setLogger(
  { verbose() {}, debug() {}, info() {}, warn: collect, error: collect },
  DiagLogLevel.WARN
);

const { AwsLambdaInstrumentation } = require('@opentelemetry/instrumentation-aws-lambda');
const { registerInstrumentations } = require('@opentelemetry/instrumentation');
const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
const { InMemorySpanExporter, SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');

const { resolveLambdaHandler } = await import('../../lambdaHandlerResolution.mjs');

/** The runtime interface client's `loadModule`, minus the loading. */
function resolveLikeTheRuntime(handlerDef) {
  const handler = path.basename(handlerDef);
  const moduleRoot = handlerDef.substring(0, handlerDef.length - handler.length);
  const moduleName = handler.substring(0, handler.indexOf('.'));
  const base = path.resolve(taskRoot, moduleRoot, moduleName);

  for (const extension of ['', '.js', '.mjs', '.cjs']) {
    const candidate = `${base}${extension}`;
    if (!fs.existsSync(candidate)) {
      continue;
    }
    if (fs.statSync(candidate).isDirectory()) {
      // `import()` of a directory throws ERR_UNSUPPORTED_DIR_IMPORT, and the runtime
      // lets that escape the loop: no further extension is tried and the fallback below
      // is never reached. The function fails at init.
      return { base, resolved: undefined, error: 'ERR_UNSUPPORTED_DIR_IMPORT' };
    }
    return { base, resolved: candidate, error: null };
  }

  try {
    return {
      base,
      resolved: require.resolve(moduleName, {
        paths: [taskRoot, path.join(taskRoot, moduleRoot)],
      }),
      error: null,
    };
  } catch (e) {
    return { base, resolved: undefined, error: e.code || String(e).split('\n')[0] };
  }
}

const memoryExporter = new InMemorySpanExporter();
const tracerProvider = new NodeTracerProvider({
  spanProcessors: [new SimpleSpanProcessor(memoryExporter)],
});

// Mirrors `opt/node/init.mjs`: an empty config without the fix, and a corrected
// `lambdaHandler` with it.
const lambdaHandler = scenario.applyFix ? resolveLambdaHandler() : undefined;

registerInstrumentations({
  instrumentations: [new AwsLambdaInstrumentation(lambdaHandler ? { lambdaHandler } : {})],
  tracerProvider,
});

const { base, resolved, error } = resolveLikeTheRuntime(scenario.handler);

let statusCode = null;
let loadError = error;
if (resolved) {
  try {
    const userModule = require(resolved);
    const functionName = path.basename(scenario.handler).split('.').slice(1).join('.');
    const result = await userModule[functionName](
      { hello: 'world' },
      {
        functionName: 'handler-resolution-test',
        functionVersion: '$LATEST',
        invokedFunctionArn:
          'arn:aws:lambda:eu-central-1:123456789012:function:handler-resolution-test',
        awsRequestId: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
      }
    );
    statusCode = result.statusCode;
  } catch (e) {
    loadError = e.code || String(e).split('\n')[0];
  }
}

await tracerProvider.forceFlush();

process.stdout.write(
  JSON.stringify({
    taskRoot,
    base,
    loadedFrom: resolved ?? null,
    loadError: loadError ?? null,
    statusCode,
    lambdaHandler: lambdaHandler ?? null,
    spans: memoryExporter.getFinishedSpans().map(span => ({
      name: span.name,
      scope: span.instrumentationScope.name,
    })),
    diag: diagMessages,
  })
);

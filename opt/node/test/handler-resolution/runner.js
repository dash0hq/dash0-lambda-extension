'use strict';

/*
 * Child-process runner for `opt/node/test/handler-resolution.test.ts`.
 *
 * Each scenario has to run in its own process: `AwsLambdaInstrumentation.init()`
 * reads `LAMBDA_TASK_ROOT` / `_HANDLER` during construction, and the require hook
 * it installs is global and caches loaded modules. Running the scenarios in-process
 * would leak state between them.
 *
 * The runner models the Lambda runtime interface faithfully, which is the point of
 * the whole exercise: the RIC derives the module path from `_HANDLER` the same way
 * the instrumentation does, then hands that path to Node's resolver. The two cannot
 * disagree about the path -- only about how it is turned into a file. Node resolves
 * directories and package `main` fields; the instrumentation only stats `.js`, `.mjs`
 * and `.cjs`. Every divergence lives in that gap.
 *
 * Usage: node runner.js '<scenario JSON>'
 *
 * Scenario:
 *   handler        value of the `_HANDLER` env var, i.e. what AWS is configured with
 *   files          { relative path inside the deployment package: file contents }
 *   lambdaHandler  optional override passed to AwsLambdaInstrumentation's config
 *
 * Prints a JSON result on stdout:
 *   { taskRoot, modulePath, loadedFrom, loadError, statusCode, spans, diag }
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

const scenario = JSON.parse(process.argv[2]);

// Build a throwaway deployment package.
const taskRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'lambda-task-root-'));
Object.keys(scenario.files).forEach(function (relative) {
  const file = path.join(taskRoot, relative);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, scenario.files[relative]);
});

process.env.LAMBDA_TASK_ROOT = taskRoot;
process.env._HANDLER = scenario.handler;
process.env.AWS_EXECUTION_ENV = 'AWS_Lambda_nodejs24.x';
process.env.AWS_LAMBDA_FUNCTION_NAME = 'syncjob-service-staging';

const { diag, DiagLogLevel } = require('@opentelemetry/api');

// Capture what the extension would print to the function's CloudWatch log group, so the
// test can assert on the exact line the customer reported.
const diagMessages = [];
const collect = (...args) => diagMessages.push(args.map(String).join(' '));
diag.setLogger(
  { verbose() {}, debug() {}, info() {}, warn: collect, error: collect },
  DiagLogLevel.WARN,
);

const { AwsLambdaInstrumentation } = require('@opentelemetry/instrumentation-aws-lambda');
const { registerInstrumentations } = require('@opentelemetry/instrumentation');
const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
const { InMemorySpanExporter, SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');

const memoryExporter = new InMemorySpanExporter();
const tracerProvider = new NodeTracerProvider({
  spanProcessors: [new SimpleSpanProcessor(memoryExporter)],
});

// Mirrors `opt/node/init.mjs`, which constructs the instrumentation with an empty config
// and so never passes a handler path of its own.
const instrumentationConfig = scenario.lambdaHandler ? { lambdaHandler: scenario.lambdaHandler } : {};

registerInstrumentations({
  instrumentations: [new AwsLambdaInstrumentation(instrumentationConfig)],
  tracerProvider,
});

(async () => {
  // What the runtime interface does: resolve(appRoot, moduleRoot, module), then require.
  const handlerName = path.basename(scenario.handler);
  const moduleRoot = scenario.handler.substring(0, scenario.handler.length - handlerName.length);
  const parts = handlerName.split('.', 2);
  const modulePath = path.resolve(taskRoot, moduleRoot, parts[0]);
  const functionName = parts[1];

  let userModule = null;
  let loadedFrom = null;
  let loadError = null;
  try {
    userModule = require(modulePath);
    loadedFrom = require.resolve(modulePath);
  } catch (err) {
    // A function whose handler cannot be loaded never returns 200 — it fails the
    // invocation with Runtime.ImportModuleError. Recorded rather than thrown so the
    // test can assert on it.
    loadError = err.code || String(err).split('\n')[0];
  }

  let statusCode = null;
  if (userModule && typeof userModule[functionName] === 'function') {
    const result = await userModule[functionName](
      { hello: 'world' },
      {
        functionName: 'syncjob-service-staging',
        functionVersion: '$LATEST',
        invokedFunctionArn: 'arn:aws:lambda:us-east-1:123456789012:function:syncjob-service-staging',
        awsRequestId: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
      },
    );
    statusCode = result.statusCode;
  }

  await tracerProvider.forceFlush();

  process.stdout.write(
    JSON.stringify({
      taskRoot: taskRoot,
      modulePath: modulePath,
      loadedFrom: loadedFrom,
      loadError: loadError,
      statusCode: statusCode,
      spans: memoryExporter.getFinishedSpans().map((span) => ({
        name: span.name,
        scope: span.instrumentationScope.name,
      })),
      diag: diagMessages,
    }),
  );
})().catch((err) => {
  process.stderr.write(String((err && err.stack) || err));
  process.exit(1);
});

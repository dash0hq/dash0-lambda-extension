'use strict';

/*
 * Child-process runner for `opt/node/test/handler-resolution.test.ts`.
 *
 * Each scenario has to run in its own process: `AwsLambdaInstrumentation.init()`
 * reads `LAMBDA_TASK_ROOT` / `_HANDLER` during construction, and the require hook
 * it installs is global and caches loaded modules. Running the scenarios in-process
 * would leak state between them.
 *
 * Usage: node runner.js '<scenario JSON>'
 *
 * Scenario:
 *   handler       value of the `_HANDLER` env var, i.e. what AWS is configured with
 *   bundlePath    where the handler file actually lands in the deployment package,
 *                 relative to LAMBDA_TASK_ROOT
 *   lambdaHandler optional override passed to AwsLambdaInstrumentation's config
 *
 * Prints a JSON result on stdout:
 *   { statusCode, spans: [{ name, scope }], diag: [...] }
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

const scenario = JSON.parse(process.argv[2]);

// Build a throwaway deployment package. `bundlePath` is deliberately independent of
// `handler` so we can model a bundler that flattens its output while the configured
// handler string keeps the source directory.
const taskRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'lambda-task-root-'));
const bundleFile = path.join(taskRoot, scenario.bundlePath);
fs.mkdirSync(path.dirname(bundleFile), { recursive: true });
fs.writeFileSync(bundleFile, 'exports.handler = async () => ({ statusCode: 200, body: "ok" });\n');

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
  // The Lambda runtime loads the user module from where the package actually put it,
  // by absolute path and without an extension.
  const userModule = require(bundleFile.replace(/\.[cm]?js$/, ''));

  const result = await userModule[scenario.handler.split('.').pop()](
    { hello: 'world' },
    {
      functionName: 'syncjob-service-staging',
      functionVersion: '$LATEST',
      invokedFunctionArn: 'arn:aws:lambda:eu-west-1:123456789012:function:syncjob-service-staging',
      awsRequestId: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
    },
  );

  await tracerProvider.forceFlush();

  process.stdout.write(
    JSON.stringify({
      taskRoot,
      statusCode: result.statusCode,
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

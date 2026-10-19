/*
 * Child-process runner for `opt/node/test/custom-metrics.test.ts`.
 *
 * Runs the real `opt/node/init.mjs` (what `/opt/wrapper` loads with `--import`) against a
 * fake extension, then invokes a handler that records metrics through its *own* copy of
 * `@opentelemetry/api` -- the distro bundles its copy into the layer, so user code never
 * shares the module instance, only the global registry.
 *
 * Lambda freezes the execution environment as soon as the handler returns, so whatever has
 * not reached the extension by then is lost (or arrives one invocation late). The runner
 * therefore snapshots the received requests right after the handler resolves, before
 * anything else gets a chance to flush.
 *
 * Usage: node runner.mjs '<scenario JSON>'
 *
 *   taskRoot   the deployment package, already on disk
 *   handler    value of `_HANDLER`
 *   env        optional extra environment variables, set before `init.mjs` is loaded
 *
 * Prints a JSON result on stdout, on a line of its own prefixed with `RESULT ` (the distro
 * logs to stdout too):
 *   { statusCode, meterProvider, atHandlerExit: [{ path, metricNames, temporalities }] }
 *
 * `temporalities` holds the OTLP `aggregationTemporality` of every exported metric
 * (1 = delta, 2 = cumulative); it is only filled for the JSON-encoded `/v1/metrics` payloads.
 */

import http from 'http';
import path from 'path';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);
const scenario = JSON.parse(process.argv[2]);
const METRIC_NAMES = ['dash0.test.custom_counter', 'dash0.test.custom_histogram'];

const received = [];

function temporalitiesOf(body) {
  try {
    return JSON.parse(body).resourceMetrics.flatMap(rm =>
      rm.scopeMetrics.flatMap(sm =>
        sm.metrics.map(m => (m.sum ?? m.histogram ?? m.exponentialHistogram)?.aggregationTemporality)
      )
    );
  } catch {
    return [];
  }
}
const server = http.createServer((req, res) => {
  const chunks = [];
  req.on('data', chunk => chunks.push(chunk));
  req.on('end', () => {
    // Protobuf and JSON both carry metric names as plain bytes, so a substring match works
    // regardless of the exporter's encoding.
    const body = Buffer.concat(chunks).toString('latin1');
    received.push({
      path: req.url,
      metricNames: METRIC_NAMES.filter(name => body.includes(name)),
      temporalities: req.url === '/v1/metrics' ? temporalitiesOf(body) : [],
    });
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end('{}');
  });
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const { port } = server.address();

process.env.LAMBDA_TASK_ROOT = scenario.taskRoot;
process.env._HANDLER = scenario.handler;
process.env.AWS_EXECUTION_ENV = 'AWS_Lambda_nodejs24.x';
process.env.AWS_LAMBDA_FUNCTION_NAME = 'custom-metrics-test';
process.env.DASH0_EXTENSION_ENDPOINT = `http://127.0.0.1:${port}/v1/traces`;
Object.assign(process.env, scenario.env);

await import('../../init.mjs');

const moduleName = path.basename(scenario.handler).split('.')[0];
const functionName = path.basename(scenario.handler).split('.').slice(1).join('.');
const userModule = require(path.join(scenario.taskRoot, moduleName));
const result = await userModule[functionName](
  {},
  {
    functionName: 'custom-metrics-test',
    functionVersion: '$LATEST',
    invokedFunctionArn: 'arn:aws:lambda:eu-central-1:123456789012:function:custom-metrics-test',
    awsRequestId: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
  }
);

const atHandlerExit = received.map(r => ({ ...r }));

process.stdout.write(
  '\nRESULT ' +
  JSON.stringify({
    statusCode: result.statusCode,
    meterProvider: result.meterProvider,
    atHandlerExit,
  }) +
  '\n'
);

// Periodic readers and batch processors keep timers alive; the frozen sandbox would too.
process.exit(0);

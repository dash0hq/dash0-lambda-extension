/*
 * Reproduction and regression test for SIG-708: metrics recorded through the OpenTelemetry
 * API in a Node function never reach the extension.
 *
 * The distro only sets up tracing. Nothing registers a global MeterProvider, so
 * `metrics.getMeter()` in user code returns a no-op meter, and `init.mjs` passes no meter
 * provider to `AwsLambdaInstrumentation`, so nothing would be flushed at handler exit
 * either. The customer workaround is a hand-built MeterProvider plus `forceFlush()` in
 * every handler.
 *
 * The handler below does what a customer does: depend on its own `@opentelemetry/api`,
 * record a counter and a histogram, and return -- no explicit flush. It runs in its own
 * process (see `custom-metrics/runner.mjs`) because `init.mjs` installs global hooks and
 * registers OpenTelemetry globals.
 */

import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const ROOT = path.join(__dirname, '..');
const RUNNER = path.join(__dirname, 'custom-metrics', 'runner.mjs');

const HANDLER_SOURCE = `
const { metrics } = require('@opentelemetry/api');

const meter = metrics.getMeter('dash0-custom-metrics-test');
const counter = meter.createCounter('dash0.test.custom_counter');
const histogram = meter.createHistogram('dash0.test.custom_histogram');

exports.handler = async () => {
  counter.add(1);
  histogram.record(42);
  return { statusCode: 200, meterProvider: metrics.getMeterProvider().constructor.name };
};
`;

// OTLP `aggregationTemporality` values.
const DELTA = 1;
const CUMULATIVE = 2;

interface Result {
  statusCode: number;
  meterProvider: string;
  atHandlerExit: { path: string; metricNames: string[]; temporalities: number[] }[];
}

let taskRoot: string;

function invoke(env: Record<string, string> = {}): Result {
  const stdout = execFileSync(
    process.execPath,
    [RUNNER, JSON.stringify({ taskRoot, handler: 'index.handler', env })],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }
  );
  const line = stdout.split('\n').find(l => l.startsWith('RESULT '));
  if (!line) throw new Error(`runner printed no result:\n${stdout}`);
  return JSON.parse(line.slice('RESULT '.length));
}

const metricRequests = (result: Result) => result.atHandlerExit.filter(r => r.path === '/v1/metrics');

beforeAll(() => {
  // `init.mjs` imports the compiled distro; CI runs `npm test` without `npm run build`.
  execFileSync(
    process.execPath,
    [require.resolve('typescript/bin/tsc'), '-p', 'distro/tsconfig.json', '--skipLibCheck'],
    { cwd: ROOT, stdio: 'inherit' }
  );

  taskRoot = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'lambda-task-root-')));
  fs.writeFileSync(path.join(taskRoot, 'index.js'), HANDLER_SOURCE);
  // The user's own copy of the API, as it would be bundled into their deployment package.
  fs.cpSync(
    path.join(ROOT, 'node_modules', '@opentelemetry', 'api'),
    path.join(taskRoot, 'node_modules', '@opentelemetry', 'api'),
    { recursive: true }
  );
}, 120_000);

afterAll(() => {
  fs.rmSync(taskRoot, { recursive: true, force: true });
});

describe('custom metrics from the OpenTelemetry API', () => {
  describe('with the default configuration', () => {
    let result: Result;

    beforeAll(() => {
      result = invoke();
    });

    it('invokes the handler and flushes its span (harness sanity check)', () => {
      expect(result.statusCode).toBe(200);
      expect(result.atHandlerExit.map(r => r.path)).toContain('/v1/traces');
    });

    it("registers a global MeterProvider that the handler's own API copy picks up", () => {
      expect(result.meterProvider).not.toBe('NoopMeterProvider');
    });

    it('exports the metrics to the extension before the handler returns', () => {
      expect(metricRequests(result).flatMap(r => r.metricNames).sort()).toEqual([
        'dash0.test.custom_counter',
        'dash0.test.custom_histogram',
      ]);
    });

    it('uses the OpenTelemetry default, cumulative temporality', () => {
      expect(metricRequests(result).flatMap(r => r.temporalities)).toEqual([CUMULATIVE, CUMULATIVE]);
    });
  });

  it('honors OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE', () => {
    const result = invoke({ OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE: 'delta' });

    expect(metricRequests(result).flatMap(r => r.temporalities)).toEqual([DELTA, DELTA]);
  });
});

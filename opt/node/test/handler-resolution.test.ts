/*
 * Reproduction for a customer report: functions are instrumented and keep returning 200,
 * but no handler spans arrive. Their CloudWatch log group shows
 *
 *   @opentelemetry/instrumentation-aws-lambda No handler file was able to resolved with
 *   one of the known extensions for the file /var/task/syncjob-service-staging/index
 *
 * on nodejs24.x, alongside healthy application logs and successful invocations.
 *
 * This is instrumentation-only — the Rust extension never reads the handler string, and
 * `opt/node/init.mjs` does not either: it constructs `AwsLambdaInstrumentation` with an
 * empty config and leaves handler resolution entirely to upstream.
 *
 * The key constraint, and what makes the failure specific: the Lambda runtime interface
 * derives the module path from `_HANDLER` exactly as the instrumentation does —
 *
 *   path.resolve(LAMBDA_TASK_ROOT, dirname(_HANDLER), basename(_HANDLER).split('.')[0])
 *
 * — so the two can never disagree about the *path*. They disagree about how that path
 * becomes a file. The runtime hands it to Node's resolver, which also resolves a
 * directory (via its `index.js` or its package `main`). The instrumentation only stats
 * `.js`, `.mjs` and `.cjs` at exactly that path, and the resolved filename is what the
 * require hook matches on. When all three stats miss, the hook is armed on an
 * extensionless path that is never loaded: the handler is never wrapped, the invocation
 * succeeds, and no span is produced.
 *
 * That gap is the bug, and it is why the customer's function still works. A genuinely
 * missing bundle is NOT this case — it fails the invocation outright, which the negative
 * control below pins down.
 *
 * Every scenario runs in its own process; see `handler-resolution/runner.js`.
 */

import { execFileSync } from 'child_process';
import * as path from 'path';

const HANDLER_SOURCE = 'exports.handler = async () => ({ statusCode: 200, body: "ok" });\n';

interface Scenario {
  /** Value of `_HANDLER`, i.e. what the function is configured with in AWS. */
  handler: string;
  /** The deployment package: path inside it -> contents. */
  files: Record<string, string>;
  /** Optional `lambdaHandler` override for the instrumentation config. */
  lambdaHandler?: string;
}

interface Result {
  taskRoot: string;
  modulePath: string;
  loadedFrom: string | null;
  loadError: string | null;
  statusCode: number | null;
  spans: { name: string; scope: string }[];
  diag: string[];
}

const RUNNER = path.join(__dirname, 'handler-resolution', 'runner.js');
const HANDLER_SCOPE = '@opentelemetry/instrumentation-aws-lambda';
const RESOLUTION_FAILURE = 'No handler file was able to resolved with one of the known extensions';
const SPAN = [{ name: 'syncjob-service-staging', scope: HANDLER_SCOPE }];

function invoke(scenario: Scenario): Result {
  const stdout = execFileSync(process.execPath, [RUNNER, JSON.stringify(scenario)], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  return JSON.parse(stdout);
}

describe('aws-lambda handler file resolution', () => {
  it('produces a span for a flat handler at the package root', () => {
    const result = invoke({
      handler: 'index.handler',
      files: { 'index.js': HANDLER_SOURCE },
    });

    expect(result.loadError).toBeNull();
    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual(SPAN);
  });

  it('produces a span when a directory prefix matches the bundle layout', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      files: { 'syncjob-service-staging/index.js': HANDLER_SOURCE },
    });

    expect(result.loadError).toBeNull();
    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual(SPAN);
  });

  // The customer's case. Currently failing — this is the bug being reproduced.
  it('produces a span when the handler resolves to a directory module', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      files: { 'syncjob-service-staging/index/index.js': HANDLER_SOURCE },
    });

    // The function is entirely healthy: Node resolves the directory to its index.js,
    // the handler runs, the caller gets 200. This is what the customer observes.
    expect(result.loadError).toBeNull();
    expect(result.loadedFrom).toBe(path.join(result.taskRoot, 'syncjob-service-staging', 'index', 'index.js'));
    expect(result.statusCode).toBe(200);

    // The customer's CloudWatch line, reproduced: the stats look for a file, not a directory.
    expect(result.diag.join('\n')).toContain(
      `${RESOLUTION_FAILURE} for the file ${path.join(result.taskRoot, 'syncjob-service-staging', 'index')}`,
    );

    // ...and the actual symptom: no telemetry from a function that works.
    expect(result.spans).toEqual(SPAN);
  });

  // Same root cause, second shape — worth covering because the directory need not
  // contain an index.js for Node to resolve it.
  it('produces a span when the handler resolves through a package main field', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      files: {
        'syncjob-service-staging/index/package.json': JSON.stringify({ main: 'app.js' }),
        'syncjob-service-staging/index/app.js': HANDLER_SOURCE,
      },
    });

    expect(result.loadError).toBeNull();
    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual(SPAN);
  });

  // Negative control. A bundle that genuinely isn't where the handler points produces
  // the same warning but is NOT what the customer is seeing: the runtime cannot load
  // the module either, so the invocation fails outright rather than returning 200.
  // This is what rules out the "bundler flattened the output" theory.
  it('fails the invocation outright when the bundle really is missing', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      files: { 'index.js': HANDLER_SOURCE },
    });

    expect(result.loadError).toBe('MODULE_NOT_FOUND');
    expect(result.statusCode).toBeNull();
    expect(result.diag.join('\n')).toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual([]);
  });

  // Not a fix, just evidence for one: upstream already accepts a `lambdaHandler` config
  // value that replaces `_HANDLER` for resolution purposes. `init.mjs` passes an empty
  // config today, so there is no way to correct the path from the outside. Plumbing an
  // env var (e.g. DASH0_LAMBDA_HANDLER) through to this config would unblock customers
  // whose handler resolves by a mechanism the three stats don't replicate.
  it('resolves the handler when the real file is passed through the config', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      files: { 'syncjob-service-staging/index/index.js': HANDLER_SOURCE },
      lambdaHandler: 'syncjob-service-staging/index/index.handler',
    });

    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual(SPAN);
  });
});

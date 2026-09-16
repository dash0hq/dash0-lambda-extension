/*
 * Reproduction for a customer report: functions are instrumented and keep returning 200,
 * but no handler spans arrive. Their CloudWatch log group shows
 *
 *   @opentelemetry/instrumentation-aws-lambda No handler file was able to resolved with
 *   one of the known extensions for the file /var/task/syncjob-service-staging/index
 *
 * on Node 24 with a handler configured as `index.handler`.
 *
 * This is instrumentation-only — the Rust extension never reads the handler string, and
 * `opt/node/init.mjs` does not either: it constructs `AwsLambdaInstrumentation` with an
 * empty config and leaves handler resolution entirely to upstream.
 *
 * Upstream resolves the handler file as
 *
 *   path.resolve(LAMBDA_TASK_ROOT, dirname(_HANDLER), basename(_HANDLER).split('.')[0])
 *
 * and then tries `.js`, `.mjs` and `.cjs` at exactly that path. The resolved filename is
 * what the require hook matches on, so when nothing is found the hook is registered for a
 * path that is never loaded: the handler is never wrapped, the invocation succeeds, and
 * no span is produced.
 *
 * The scenarios below show that a directory prefix in the handler string is not itself the
 * problem. It only breaks when the handler string points somewhere the bundle is not —
 * the shape a bundler produces when it flattens its output (e.g. CDK's `NodejsFunction`
 * emitting `index.js` at the package root) while the configured handler keeps the source
 * directory.
 *
 * Every scenario runs in its own process; see `handler-resolution/runner.js`.
 */

import { execFileSync } from 'child_process';
import * as path from 'path';

interface Scenario {
  /** Value of `_HANDLER`, i.e. what the function is configured with in AWS. */
  handler: string;
  /** Where the handler file actually lands in the deployment package. */
  bundlePath: string;
  /** Optional `lambdaHandler` override for the instrumentation config. */
  lambdaHandler?: string;
}

interface Result {
  taskRoot: string;
  statusCode: number;
  spans: { name: string; scope: string }[];
  diag: string[];
}

const RUNNER = path.join(__dirname, 'handler-resolution', 'runner.js');
const HANDLER_SCOPE = '@opentelemetry/instrumentation-aws-lambda';
const RESOLUTION_FAILURE = 'No handler file was able to resolved with one of the known extensions';

function invoke(scenario: Scenario): Result {
  const stdout = execFileSync(process.execPath, [RUNNER, JSON.stringify(scenario)], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  return JSON.parse(stdout);
}

describe('aws-lambda handler file resolution', () => {
  it('produces a span for a flat handler at the package root', () => {
    const result = invoke({ handler: 'index.handler', bundlePath: 'index.js' });

    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual([{ name: 'syncjob-service-staging', scope: HANDLER_SCOPE }]);
  });

  it('produces a span when a directory prefix matches the bundle layout', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      bundlePath: 'syncjob-service-staging/index.js',
    });

    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual([{ name: 'syncjob-service-staging', scope: HANDLER_SCOPE }]);
  });

  // The customer's case. Currently failing — this is the bug being reproduced.
  it('produces a span when the handler prefix does not match the bundle layout', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      bundlePath: 'index.js',
    });

    // The invocation itself is unaffected, which is why this surfaces as silence rather
    // than as an error.
    expect(result.statusCode).toBe(200);

    // The customer's CloudWatch line, reproduced.
    expect(result.diag.join('\n')).toContain(
      `${RESOLUTION_FAILURE} for the file ${path.join(result.taskRoot, 'syncjob-service-staging', 'index')}`,
    );

    // ...and the actual symptom: no telemetry.
    expect(result.spans).toEqual([{ name: 'syncjob-service-staging', scope: HANDLER_SCOPE }]);
  });

  // Not a fix, just evidence for one: upstream already accepts a `lambdaHandler` config
  // value that replaces `_HANDLER` for resolution purposes. `init.mjs` passes an empty
  // config today, so there is no way to correct the path from the outside. Plumbing an
  // env var (e.g. DASH0_LAMBDA_HANDLER) through to this config would unblock customers
  // who cannot change their handler string or bundle layout.
  it('resolves the handler when the correct path is passed through the config', () => {
    const result = invoke({
      handler: 'syncjob-service-staging/index.handler',
      bundlePath: 'index.js',
      lambdaHandler: 'index.handler',
    });

    expect(result.statusCode).toBe(200);
    expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
    expect(result.spans).toEqual([{ name: 'syncjob-service-staging', scope: HANDLER_SCOPE }]);
  });
});

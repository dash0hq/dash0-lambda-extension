/*
 * Reproduction and regression test for a customer report: functions are instrumented and
 * keep returning 200, but no handler spans arrive. Their log group shows
 *
 *   @opentelemetry/instrumentation-aws-lambda No handler file was able to resolved with
 *   one of the known extensions for the file /var/task/foo/index
 *
 * on nodejs24.x, alongside healthy application logs and successful invocations. Their
 * deployment package contains `index.js` at the root and nothing else; the function is
 * configured with a handler carrying a directory prefix that is not in the package.
 *
 * This is instrumentation-only. The Rust extension never reads the handler string, and
 * neither did `opt/node/init.mjs` before the fix under test: it constructed
 * `AwsLambdaInstrumentation` with an empty config and left resolution to upstream.
 *
 * The gap: the Lambda runtime resolves the handler module in five steps, and upstream
 * replicates three of them. When the runtime uses one of the other two, the
 * instrumentation arms require-in-the-middle on a path that is never loaded -- so the
 * handler is never wrapped, the invocation succeeds, and no span is produced. Nothing
 * downstream treats this as an error, because a hook that matches nothing is a normal
 * state.
 *
 * `opt/node/lambdaHandlerResolution.mjs` closes it by computing what the runtime will
 * actually load and passing it to upstream's `lambdaHandler` config option.
 *
 * Every scenario runs in its own process; see `handler-resolution/runner.mjs`, which also
 * explains what it models and where it deliberately deviates. Full write-up, including
 * how this was verified against real AWS: `handler-resolution/README.md`.
 */

import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const HANDLER_SOURCE = 'exports.handler = async () => ({ statusCode: 200, body: "ok" });\n';
const SOURCE_MAP = '{"version":3,"sources":["src/index.ts"],"mappings":"","names":[]}\n';

const RUNNER = path.join(__dirname, 'handler-resolution', 'runner.mjs');
const HANDLER_SCOPE = '@opentelemetry/instrumentation-aws-lambda';
const RESOLUTION_FAILURE = 'No handler file was able to resolved with one of the known extensions';
const SPAN = [{ name: 'handler-resolution-test', scope: HANDLER_SCOPE }];

interface Scenario {
  /** Value of `_HANDLER`, i.e. what the function is configured with in AWS. */
  handler: string;
  /** The deployment package: path inside it -> contents. */
  files: Record<string, string>;
  /** Whether to apply our handler correction, as `init.mjs` does. */
  applyFix?: boolean;
}

interface Result {
  taskRoot: string;
  base: string;
  loadedFrom: string | null;
  loadError: string | null;
  statusCode: number | null;
  lambdaHandler: string | null;
  spans: { name: string; scope: string }[];
  diag: string[];
}

const taskRoots: string[] = [];

afterAll(() => {
  taskRoots.forEach(taskRoot => fs.rmSync(taskRoot, { recursive: true, force: true }));
});

function invoke({ handler, files, applyFix = false }: Scenario): Result {
  // `realpathSync` because macOS hands out /var/folders symlinks, and the runner compares
  // resolved absolute paths.
  const taskRoot = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'lambda-task-root-')));
  taskRoots.push(taskRoot);

  Object.entries(files).forEach(([relative, contents]) => {
    const file = path.join(taskRoot, relative);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, contents);
  });

  const stdout = execFileSync(
    process.execPath,
    [RUNNER, JSON.stringify({ taskRoot, handler, applyFix })],
    {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'inherit'],
      // Lambda puts the task root on NODE_PATH. Node reads it once at startup, which is
      // why it has to be set here rather than inside the runner -- and why this failure
      // cannot be reproduced without it.
      env: { ...process.env, NODE_PATH: taskRoot },
    }
  );

  return JSON.parse(stdout);
}

describe('aws-lambda handler file resolution', () => {
  describe('when upstream resolves the handler correctly', () => {
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
        handler: 'foo/index.handler',
        files: { 'foo/index.js': HANDLER_SOURCE },
      });

      expect(result.loadError).toBeNull();
      expect(result.statusCode).toBe(200);
      expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
      expect(result.spans).toEqual(SPAN);
    });
  });

  /*
   * The customer's configuration. Verified against a real nodejs24.x function in AWS: it
   * returns 200, logs the resolution warning, and emits no spans.
   */
  describe('when the runtime resolves the handler through NODE_PATH', () => {
    const CUSTOMER_SCENARIO = {
      handler: 'foo/index.handler',
      files: { 'index.js': HANDLER_SOURCE, 'index.js.map': SOURCE_MAP },
    };

    it('reproduces the failure: the function works, and no span is produced', () => {
      const result = invoke(CUSTOMER_SCENARIO);

      // The function is entirely healthy. The handler string's `foo/` prefix does not
      // exist in the package and is simply ignored: the runtime's bare-specifier
      // fallback resolves `index` through NODE_PATH, which contains the task root.
      expect(result.loadError).toBeNull();
      expect(result.loadedFrom).toBe(path.join(result.taskRoot, 'index.js'));
      expect(result.statusCode).toBe(200);

      // The customer's CloudWatch line, reproduced: upstream only stats the three
      // extensions at the prefixed path.
      expect(result.diag.join('\n')).toContain(
        `${RESOLUTION_FAILURE} for the file ${path.join(result.taskRoot, 'foo', 'index')}`
      );

      // ...and the symptom: no telemetry from a function that works.
      expect(result.spans).toEqual([]);
    });

    it('is fixed by correcting the handler passed to the instrumentation', () => {
      const result = invoke({ ...CUSTOMER_SCENARIO, applyFix: true });

      expect(result.lambdaHandler).toBe('index.handler');
      expect(result.statusCode).toBe(200);
      expect(result.diag.join('\n')).not.toContain(RESOLUTION_FAILURE);
      expect(result.spans).toEqual(SPAN);
    });

    it('resolves a handler whose module lives under node_modules', () => {
      const result = invoke({
        handler: 'foo/index.handler',
        files: {
          'node_modules/index/package.json': JSON.stringify({ main: 'app.js' }),
          'node_modules/index/app.js': HANDLER_SOURCE,
        },
        applyFix: true,
      });

      expect(result.lambdaHandler).toBe(path.join('node_modules', 'index', 'app') + '.handler');
      expect(result.statusCode).toBe(200);
      expect(result.spans).toEqual(SPAN);
    });
  });

  describe('when the handler cannot be loaded at all', () => {
    /*
     * A genuinely missing module is NOT the customer's case, and this pins the difference
     * down: the runtime cannot load it either, so the invocation fails outright instead
     * of returning 200. This is what rules out the "the bundler flattened its output"
     * theory whenever the same warning shows up on a working function.
     */
    it('fails the invocation when nothing resolves', () => {
      const result = invoke({
        handler: 'foo/missing.handler',
        files: { 'index.js': HANDLER_SOURCE },
      });

      expect(result.loadError).toBe('MODULE_NOT_FOUND');
      expect(result.statusCode).toBeNull();
      expect(result.spans).toEqual([]);
    });

    /*
     * On nodejs24.x the runtime `import()`s the handler, and `import()` refuses directory
     * imports. The error escapes the extension loop, so neither the remaining extensions
     * nor the bare-specifier fallback are tried: the function dies at init. Verified in
     * AWS -- `Runtime.Unknown`, `ERR_UNSUPPORTED_DIR_IMPORT`, every invocation.
     */
    it('fails the invocation when the handler path is a directory', () => {
      const result = invoke({
        handler: 'foo/index.handler',
        files: { 'foo/index/index.js': HANDLER_SOURCE },
      });

      expect(result.loadError).toBe('ERR_UNSUPPORTED_DIR_IMPORT');
      expect(result.statusCode).toBeNull();
      expect(result.spans).toEqual([]);
    });

    it('does not invent a handler correction when nothing resolves', () => {
      const result = invoke({
        handler: 'foo/missing.handler',
        files: { 'index.js': HANDLER_SOURCE },
        applyFix: true,
      });

      expect(result.lambdaHandler).toBeNull();
      expect(result.spans).toEqual([]);
    });
  });
});

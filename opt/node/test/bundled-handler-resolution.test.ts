/*
 * `handler-resolution.test.ts` exercises `lambdaHandlerResolution.mjs` as source. Lambda
 * runs it bundled, and the bundler can change what it does: webpack 5.112 compiled
 * `createRequire(file).resolve(...)` into `(void 0).resolve(...)`, so the NODE_PATH
 * fallback threw inside its `try`, returned nothing, and the stale-handler-prefix
 * functions lost their handler span while every unit test stayed green.
 *
 * This bundles the module with the real `webpack.config.mjs` and runs the output under
 * the same NODE_PATH condition. Both steps run in child processes: the config is ESM, and
 * NODE_PATH is only read at process start.
 */

import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const NODE_DIR = path.resolve(__dirname, '..');

const BUNDLE_SCRIPT = `
import webpack from 'webpack';
import config from './webpack.config.mjs';
const [entry, outDir] = process.argv.slice(1);
const stats = await new Promise((resolve, reject) =>
  webpack({ ...config, entry, output: { ...config.output, path: outDir } }, (err, s) =>
    err ? reject(err) : resolve(s)));
if (stats.hasErrors()) {
  console.error(stats.toString('errors-only'));
  process.exit(1);
}
`;

const RESOLVE_SCRIPT = `
const { resolveLambdaHandler } = await import(process.argv[1]);
process.stdout.write(JSON.stringify(resolveLambdaHandler() ?? null));
`;

describe('lambdaHandlerResolution.mjs, bundled with webpack.config.mjs', () => {
  let workDir: string;
  let bundle: string;

  beforeAll(() => {
    workDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'bundled-handler-')));
    const outDir = path.join(workDir, 'dist');
    execFileSync(
      process.execPath,
      ['--input-type=module', '-e', BUNDLE_SCRIPT, './lambdaHandlerResolution.mjs', outDir],
      { cwd: NODE_DIR, stdio: ['ignore', 'ignore', 'inherit'] }
    );
    bundle = path.join(outDir, 'init.mjs');
  }, 120_000);

  afterAll(() => {
    fs.rmSync(workDir, { recursive: true, force: true });
  });

  it('resolves a handler that the runtime only finds through NODE_PATH', () => {
    // The deployment package has `index.js` at its root; the function is configured with
    // a `dist/` prefix that does not exist, as in the stale-handler-prefix integration test.
    const taskRoot = path.join(workDir, 'task');
    fs.mkdirSync(taskRoot);
    fs.writeFileSync(path.join(taskRoot, 'index.js'), 'exports.handler = async () => "ok";\n');

    const stdout = execFileSync(
      process.execPath,
      ['--input-type=module', '-e', RESOLVE_SCRIPT, bundle],
      {
        cwd: NODE_DIR,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'inherit'],
        env: {
          ...process.env,
          LAMBDA_TASK_ROOT: taskRoot,
          _HANDLER: 'dist/index.handler',
          NODE_PATH: taskRoot,
        },
      }
    );

    expect(JSON.parse(stdout)).toBe('index.handler');
  });
});

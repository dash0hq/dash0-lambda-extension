import { createRequire } from 'module';
import fs from 'fs';
import path from 'path';
import { diag } from '@opentelemetry/api';

import { DASH0_LOGGING_NAMESPACE } from './constants';

/*
 * The AWS Lambda Node.js runtime interface client resolves the handler module in five
 * steps (`dist/function/module-loader.js`, inlined into /var/runtime/index.mjs):
 *
 *   1. `<taskRoot>/<moduleRoot>/<module>` with no extension, then `.js`, `.mjs`, `.cjs`
 *   2. failing all four, `<module>` as a *bare specifier*, resolved from `<taskRoot>` and
 *      `<taskRoot>/<moduleRoot>`
 *
 * `@opentelemetry/instrumentation-aws-lambda` replicates only the three extensions of
 * step 1. When the runtime loads the handler through either of the other two paths, the
 * instrumentation computes a filename that is never loaded and arms its require hook on
 * it. The handler is silently left unwrapped: the invocation succeeds, the function
 * behaves normally, and no span is produced. The only trace is one warning in the
 * function's own log group:
 *
 *   @opentelemetry/instrumentation-aws-lambda No handler file was able to resolved with
 *   one of the known extensions for the file /var/task/foo/index
 *
 * Step 2 is not a corner case on Lambda, because NODE_PATH there contains the task root:
 *
 *   NODE_PATH=/opt/nodejs/node24/node_modules:/opt/nodejs/node_modules:
 *             /var/runtime/node_modules:/var/runtime:/var/task
 *
 * so `require.resolve('index')` finds `/var/task/index.js` through Node's global paths. A
 * function configured as `foo/index.handler` whose deployment package contains only
 * `index.js` at the root therefore runs perfectly well -- the directory prefix is simply
 * ignored -- while the instrumentation looks for `/var/task/foo/index.*`, finds nothing,
 * and gives up. Handler prefixes left stale by a bundler are common, and because nothing
 * breaks, nobody notices until the traces are missing.
 *
 * Upstream accepts a `lambdaHandler` config value that replaces `_HANDLER` for resolution
 * purposes. We work out what the runtime will actually load, express it back as a handler
 * string, and pass it in; upstream's own resolution then lands on the right file.
 *
 * Verified against a real nodejs24.x function; see test/handler-resolution/README.md.
 */

/** Extensions the runtime interface client tries, in its order. */
const EXTENSION_LOOKUP_ORDER = ['', '.js', '.mjs', '.cjs'];

/** Extensions upstream can rediscover from a handler string we hand it. */
const UPSTREAM_RESOLVABLE_EXTENSIONS = ['.js', '.mjs', '.cjs'];

/*
 * The namespace every log line from the JS side carries, so that `DASH0_DEBUG=true` output
 * is greppable as ours.
 *
 * A component logger of our own, rather than the shared one from `./logging`: importing
 * that module installs a global diag logger as a side effect, and the plain-Node test
 * runner (`test/handler-resolution/runner.mjs`) installs its own to capture what would
 * reach the function's log group.
 *
 * The Rust extension has its own, separate convention -- `log_prefix()` in `src/main.rs`,
 * which yields `DASH0` and `DASH0:<suffix>` -- so extension lines read `[DASH0] ...`.
 * Nothing shares a namespace across the two sides.
 */
const logger = diag.createComponentLogger({ namespace: DASH0_LOGGING_NAMESPACE });

/*
 * There is no built-in `isFile` predicate. `fs.existsSync` is the closest, but it cannot
 * tell a file from a directory, and a directory has to count as a miss here -- the runtime
 * `import()`s what it finds, and `import()` rejects directories.
 *
 * `throwIfNoEntry: false` is the built-in way to handle the ordinary miss without an
 * exception. The catch covers what that flag does not: ENOTDIR when a path component is
 * itself a file, EACCES on an unreadable directory. Those must not escape -- `bootstrap.ts`
 * wraps the whole initialisation in one try/catch, so a throw here would cost every
 * instrumentation, not just this correction.
 */
function isFile(candidate: string): boolean {
  try {
    return fs.statSync(candidate, { throwIfNoEntry: false })?.isFile() ?? false;
  } catch {
    return false;
  }
}

/**
 * Resolve `module` as a bare specifier the way the runtime interface client does. The
 * anchor path need not exist; it only seeds the resolution paths, and resolution falls
 * through to NODE_PATH from there.
 */
function resolveBareSpecifier(
  taskRoot: string,
  moduleRoot: string,
  moduleName: string
): string | undefined {
  try {
    return createRequire(path.join(taskRoot, 'index.js')).resolve(moduleName, {
      paths: [taskRoot, path.join(taskRoot, moduleRoot)],
    });
  } catch (e) {
    return undefined;
  }
}

/**
 * Compute a replacement for `_HANDLER` that points at the file the Lambda runtime will
 * actually load, or `undefined` to leave upstream's own resolution untouched.
 *
 * Returns `undefined` whenever upstream already gets it right and whenever we cannot
 * improve on it, so the worst case is exactly today's behaviour.
 */
export function resolveLambdaHandler(env: NodeJS.ProcessEnv = process.env): string | undefined {
  const taskRoot = env.LAMBDA_TASK_ROOT;
  const handlerDef = env._HANDLER;

  if (!taskRoot || !handlerDef) {
    return undefined;
  }

  const handler = path.basename(handlerDef);
  const moduleRoot = handlerDef.substring(0, handlerDef.length - handler.length);
  const separator = handler.indexOf('.');
  if (separator < 1 || separator === handler.length - 1) {
    // Not a `<module>.<function>` handler string; leave it to upstream.
    return undefined;
  }
  const moduleName = handler.substring(0, separator);
  const functionName = handler.substring(separator + 1);

  const base = path.resolve(taskRoot, moduleRoot, moduleName);

  /*
   * Steps 1-4. Upstream stats `.js`, `.mjs` and `.cjs`, and when all three miss it keeps
   * the extensionless path as the filename -- which is right when an extensionless file
   * is what the runtime loads. Nothing to correct in either case.
   */
  if (EXTENSION_LOOKUP_ORDER.some(extension => isFile(`${base}${extension}`))) {
    return undefined;
  }

  // Step 5: the gap.
  const resolved = resolveBareSpecifier(taskRoot, moduleRoot, moduleName);
  if (!resolved) {
    /*
     * The runtime cannot load the handler either, so this function fails every invocation
     * with Runtime.ImportModuleError and there is nothing to instrument. Say so anyway:
     * upstream's warning collapses the task root and the handler string into a single
     * path, which is hard to act on.
     */
    logger.warn(
      'The configured Lambda handler does not resolve to any file; the function will ' +
        'fail to start, and no spans will be produced.',
      { taskRoot, handlerDef, moduleRoot, module: moduleName, searched: base }
    );
    return undefined;
  }

  const extension = path.extname(resolved);
  if (!UPSTREAM_RESOLVABLE_EXTENSIONS.includes(extension)) {
    // The correction can only be expressed as a handler string, and upstream turns that
    // back into a file by trying exactly these extensions.
    return undefined;
  }

  /*
   * A handler string names a module, not a file: `<path without extension>.<function>`,
   * relative to the task root. Rebuild it in those two steps, because upstream will undo
   * them in the same order.
   */
  const resolvedWithoutExtension = resolved.slice(0, -extension.length);
  const moduleRelativeToTaskRoot = path.relative(taskRoot, resolvedWithoutExtension);
  if (moduleRelativeToTaskRoot === '' || moduleRelativeToTaskRoot.includes('.')) {
    /*
     * A dot would be re-parsed by upstream as the module/function separator. This also
     * rules out anything resolved outside the task root (`../..`), e.g. a handler shipped
     * in a layer: a handler string cannot express that unambiguously, so leave it be.
     */
    return undefined;
  }

  const corrected = `${moduleRelativeToTaskRoot}.${functionName}`;
  logger.debug(
    'The Lambda handler resolves to a file the instrumentation would not have found; ' +
      'correcting the handler passed to the instrumentation.',
    { taskRoot, handlerDef, searched: base, resolved, lambdaHandler: corrected }
  );

  return corrected;
}

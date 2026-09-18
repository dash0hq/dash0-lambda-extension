import { createRequire } from 'module';
import fs from 'fs';
import path from 'path';
import { diag } from '@opentelemetry/api';

import { DASH0_LOGGING_NAMESPACE } from './constants';

/*
 * The Lambda runtime interface client resolves the handler module by trying
 * `<taskRoot>/<moduleRoot>/<module>` with no extension and then `.js`, `.mjs` and `.cjs`,
 * and failing all four, `<module>` as a bare specifier -- which on Lambda reaches the task
 * root through NODE_PATH. `@opentelemetry/instrumentation-aws-lambda` implements only the
 * three extensions, so when the runtime takes the bare-specifier path the instrumentation
 * arms its require hook on a file that is never loaded: the function works, and no span is
 * produced.
 *
 * Upstream accepts a `lambdaHandler` config value that replaces `_HANDLER` for resolution.
 * We work out what the runtime will actually load and express it back as a handler string,
 * so that upstream's own resolution lands on the right file.
 *
 * Full write-up, including how this was verified against a real nodejs24.x function:
 * test/handler-resolution/README.md.
 */

/** Extensions the runtime tries, in its order. The empty one is an extensionless file. */
const RUNTIME_EXTENSION_LOOKUP_ORDER = ['', '.js', '.mjs', '.cjs'];

/** Extensions upstream can rediscover from a handler string we hand it. */
const UPSTREAM_RESOLVABLE_EXTENSIONS = ['.js', '.mjs', '.cjs'];

// Not the shared logger from `./logging`: importing that module installs a global diag
// logger as a side effect, and the plain-Node test runner installs its own.
const logger = diag.createComponentLogger({ namespace: DASH0_LOGGING_NAMESPACE });

interface HandlerDefinition {
  /** The directory prefix, e.g. `foo/` in `foo/index.handler`. */
  moduleRoot: string;
  moduleName: string;
  functionName: string;
}

/** Split `<moduleRoot><module>.<function>`, or `undefined` if it is not that shape. */
function parseHandlerDefinition(handlerDef: string): HandlerDefinition | undefined {
  const handler = path.basename(handlerDef);
  const separator = handler.indexOf('.');

  if (separator < 1 || separator === handler.length - 1) {
    return undefined;
  }

  return {
    moduleRoot: handlerDef.substring(0, handlerDef.length - handler.length),
    moduleName: handler.substring(0, separator),
    functionName: handler.substring(separator + 1),
  };
}

/*
 * A directory counts as a miss: the runtime `import()`s what it finds, and `import()`
 * rejects directories. Nothing may throw -- `bootstrap.ts` wraps the whole initialisation
 * in one try/catch, so an ENOTDIR or EACCES here would cost every instrumentation, not
 * just this correction.
 */
function isFile(candidate: string): boolean {
  try {
    return fs.statSync(candidate, { throwIfNoEntry: false })?.isFile() ?? false;
  } catch {
    return false;
  }
}

/**
 * Whether upstream already arms its hook on the file the runtime loads. When all three
 * extensions miss, upstream keeps the extensionless path -- right exactly when an
 * extensionless file is what the runtime loads, hence the empty extension in the list.
 */
function upstreamAlreadyResolvesHandler(base: string): boolean {
  return RUNTIME_EXTENSION_LOOKUP_ORDER.some(extension => isFile(`${base}${extension}`));
}

/**
 * Resolve `moduleName` as a bare specifier the way the runtime does. The anchor path need
 * not exist; it only seeds the resolution paths, which fall through to NODE_PATH.
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
 * Express a resolved file back as a handler string -- `<path without extension>.<function>`
 * relative to the task root -- or `undefined` when it cannot be expressed as one.
 */
function asHandlerString(
  taskRoot: string,
  file: string,
  functionName: string
): string | undefined {
  const extension = path.extname(file);
  if (!UPSTREAM_RESOLVABLE_EXTENSIONS.includes(extension)) {
    // Upstream turns a handler string back into a file by trying exactly these.
    return undefined;
  }

  const modulePath = path.relative(taskRoot, file.slice(0, -extension.length));

  // A dot would be re-parsed by upstream as the module/function separator. This also rules
  // out anything resolved outside the task root (`../..`), e.g. a handler shipped in a
  // layer: a handler string cannot express that unambiguously.
  if (modulePath === '' || modulePath.includes('.')) {
    return undefined;
  }

  return `${modulePath}.${functionName}`;
}

/**
 * Compute a replacement for `_HANDLER` that points at the file the Lambda runtime will
 * actually load, or `undefined` to leave upstream's own resolution untouched -- both when
 * upstream already gets it right and when we cannot improve on it, so that the worst case
 * is exactly today's behaviour.
 */
export function resolveLambdaHandler(env: NodeJS.ProcessEnv = process.env): string | undefined {
  const taskRoot = env.LAMBDA_TASK_ROOT;
  const handlerDef = env._HANDLER;

  if (!taskRoot || !handlerDef) {
    return undefined;
  }

  const handler = parseHandlerDefinition(handlerDef);
  if (!handler) {
    return undefined;
  }
  const { moduleRoot, moduleName, functionName } = handler;

  const base = path.resolve(taskRoot, moduleRoot, moduleName);
  if (upstreamAlreadyResolvesHandler(base)) {
    return undefined;
  }

  const resolved = resolveBareSpecifier(taskRoot, moduleRoot, moduleName);
  if (!resolved) {
    /*
     * The runtime cannot load the handler either, so the function fails every invocation
     * and there is nothing to instrument. Say so anyway: upstream's warning collapses the
     * task root and the handler string into a single path, which is hard to act on.
     */
    logger.warn(
      'The configured Lambda handler does not resolve to any file; the function will ' +
        'fail to start, and no spans will be produced.',
      { taskRoot, handlerDef, moduleRoot, module: moduleName, searched: base }
    );
    return undefined;
  }

  const corrected = asHandlerString(taskRoot, resolved, functionName);
  if (!corrected) {
    return undefined;
  }

  logger.debug(
    'The Lambda handler resolves to a file the instrumentation would not have found; ' +
      'correcting the handler passed to the instrumentation.',
    { taskRoot, handlerDef, searched: base, resolved, lambdaHandler: corrected }
  );

  return corrected;
}

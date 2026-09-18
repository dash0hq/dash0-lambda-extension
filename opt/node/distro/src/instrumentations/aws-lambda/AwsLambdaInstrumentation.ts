import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';

import { resolveLambdaHandler } from '../../lambdaHandlerResolution';
import { TracingInstrumentor } from '../instrumentor';

/**
 * The signature of `InstrumentationBase#_onRequire`, which is `private` in the upstream
 * type declarations and so is not reachable through the public surface of the class.
 */
type OnRequireHook = (module: any, exports: any, name: string, basedir?: string) => any;

/*
 * When esbuild bundles ESM to CJS format, it defines exports using non-configurable
 * accessor descriptors. OpenTelemetry's shimmer then fails with "Cannot redefine property"
 * when trying to wrap the handler. This replaces such exports with a new object that has
 * configurable data descriptors so shimmer can wrap them.
 * See: https://github.com/evanw/esbuild/issues/2199
 */
function makeExportsConfigurable(moduleExports: any): any {
  if (typeof moduleExports !== 'object' || moduleExports === null) {
    return moduleExports;
  }
  const keys = Object.getOwnPropertyNames(moduleExports);
  const descriptors = Object.getOwnPropertyDescriptors(moduleExports);
  if (keys.every(key => descriptors[key].configurable)) {
    return moduleExports;
  }
  const fixed = Object.create(Object.getPrototypeOf(moduleExports));
  for (const key of keys) {
    Object.defineProperty(fixed, key, {
      value: moduleExports[key],
      writable: true,
      enumerable: descriptors[key].enumerable,
      configurable: true,
    });
  }
  return fixed;
}

export default class Dash0AwsLambdaInstrumentation extends TracingInstrumentor<AwsLambdaInstrumentation> {
  /**
   * Every other instrumentation here asks "is this library installed", which is what the
   * inherited implementation answers. This one does not patch a package at all: it patches
   * the function's own handler module, a path computed from `LAMBDA_TASK_ROOT` and
   * `_HANDLER`. Those two variables are set by the Lambda runtime and by nothing else, so
   * their presence is the question worth asking -- are we running in a Lambda function.
   */
  override isApplicable(): boolean {
    return Boolean(process.env.LAMBDA_TASK_ROOT && process.env._HANDLER);
  }

  /**
   * Not an npm package, unlike every other entry in this list: the module this
   * instrumentation patches is the function's handler, whose path is known only at
   * runtime. `aws-lambda` names it in the `Instrumented modules: ...` debug line, which is
   * what this list feeds. Nothing tries to require it -- `isApplicable()` above is
   * overridden precisely so that this list is never used to answer that question.
   */
  getInstrumentedModules(): string[] {
    return ['aws-lambda'];
  }

  getInstrumentation(): AwsLambdaInstrumentation {
    /*
     * Upstream resolves the handler file with three `statSync` calls, while the Lambda
     * runtime has two further resolution paths. When the runtime uses one of them the
     * instrumentation hooks a file that is never loaded, and the handler is never wrapped:
     * the function returns normally and no span is produced. `resolveLambdaHandler`
     * returns a corrected handler string in exactly that case, and `undefined` otherwise
     * -- see `lambdaHandlerResolution.ts`.
     */
    const lambdaHandler = resolveLambdaHandler();

    const instrumentation = new AwsLambdaInstrumentation(lambdaHandler ? { lambdaHandler } : {});

    /*
     * Override `_onRequire` on the instance to fix non-configurable exports before
     * patching (see `makeExportsConfigurable` above). We can't override `init()` because
     * it's already called during construction (via `enable()`). `_onRequire` is called
     * lazily when modules are required, so this override takes effect before the Lambda
     * runtime loads the handler.
     */
    const patchable = instrumentation as unknown as { _onRequire: OnRequireHook };
    const originalOnRequire = patchable._onRequire;
    patchable._onRequire = function (module, exports, name, basedir) {
      return originalOnRequire.call(this, module, makeExportsConfigurable(exports), name, basedir);
    };

    return instrumentation;
  }
}

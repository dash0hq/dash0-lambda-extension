import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';

import { resolveLambdaHandler } from '../../lambdaHandlerResolution';
import { TracingInstrumentor } from '../instrumentor';

/** `InstrumentationBase#_onRequire`, which upstream declares `private`. */
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

/*
 * Nothing here is Lambda-specific -- `_onRequire` comes from `InstrumentationBase`, so any
 * instrumentation could do this. Only the handler needs it: it is the one module that is
 * both esbuild-bundled and still reached through the require hook. A library bundled the
 * same way is not patched at all, because no `require` of it survives the bundling.
 */
function fixExportsBeforePatching(instrumentation: AwsLambdaInstrumentation): void {
  // `_onRequire` rather than `init()`: `init()` has already run by the time the constructor
  // returns, whereas `_onRequire` runs when the runtime loads the handler.
  const patchable = instrumentation as unknown as { _onRequire: OnRequireHook };
  const originalOnRequire = patchable._onRequire;
  patchable._onRequire = function (module, exports, name, basedir) {
    return originalOnRequire.call(this, module, makeExportsConfigurable(exports), name, basedir);
  };
}

export default class Dash0AwsLambdaInstrumentation extends TracingInstrumentor<AwsLambdaInstrumentation> {
  // Not the inherited "is the package installed": this patches the function's handler
  // module, a path computed from `_HANDLER`. Both variables are set by the Lambda runtime
  // and by nothing else, so together they answer "are we running in a Lambda function".
  override isApplicable(): boolean {
    return Boolean(process.env.LAMBDA_TASK_ROOT && process.env._HANDLER);
  }

  // Not an npm package: it only labels this entry in the `Instrumented modules: ...` line.
  // Nothing requires it -- that is what the `isApplicable()` override above is for.
  getInstrumentedModules(): string[] {
    return ['aws-lambda'];
  }

  getInstrumentation(): AwsLambdaInstrumentation {
    // Upstream locates the handler file by trying three extensions, where the runtime has
    // two further resolution paths. `resolveLambdaHandler` returns a corrected handler
    // string when the runtime would load something upstream cannot find, else `undefined`.
    const lambdaHandler = resolveLambdaHandler();

    const instrumentation = new AwsLambdaInstrumentation(lambdaHandler ? { lambdaHandler } : {});
    fixExportsBeforePatching(instrumentation);

    return instrumentation;
  }
}

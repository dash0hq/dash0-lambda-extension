import * as dash0 from "./distro/dist/src/distro.js";
import {AwsLambdaInstrumentation} from '@opentelemetry/instrumentation-aws-lambda';
import {registerInstrumentations} from '@opentelemetry/instrumentation';
import {register} from "module";
import {resolveLambdaHandler} from "./lambdaHandlerResolution.mjs";

try {

// When esbuild bundles ESM to CJS format, it defines exports using non-configurable
// accessor descriptors. OpenTelemetry's shimmer then fails with "Cannot redefine property"
// when trying to wrap the handler. This replaces such exports with a new object that has
// configurable data descriptors so shimmer can wrap them.
// See: https://github.com/evanw/esbuild/issues/2199
    function makeExportsConfigurable(moduleExports) {
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

// Upstream resolves the handler file with three `statSync` calls, while the Lambda
// runtime has two further resolution paths. When the runtime uses one of them the
// instrumentation hooks a file that is never loaded, and the handler is never wrapped:
// the function returns normally and no span is produced. `resolveLambdaHandler` returns
// a corrected handler string in exactly that case, and `undefined` otherwise -- see
// `lambdaHandlerResolution.mjs`.
    const lambdaHandler = resolveLambdaHandler();

    const awsLambdaInstrumentation = new AwsLambdaInstrumentation(
        lambdaHandler ? {lambdaHandler} : {}
    );

// Override _onRequire on the instance to fix non-configurable exports before patching.
// We can't override init() because it's already called during construction (via enable()).
// _onRequire is called lazily when modules are required, so this override takes effect
// before the Lambda runtime loads the handler.
    const originalOnRequire = awsLambdaInstrumentation._onRequire;
    awsLambdaInstrumentation._onRequire = function (module, exports, name, basedir) {
        return originalOnRequire.call(this, module, makeExportsConfigurable(exports), name, basedir);
    };

    const tracerProvider = (await dash0.init).tracerProvider;

    registerInstrumentations({
        instrumentations: [
            awsLambdaInstrumentation
        ],
        tracerProvider
    });

    register('import-in-the-middle/hook.mjs', import.meta.url);

} catch (err) {
    console.error('Error initializing Dash0 tracer:', err);
}
import * as lumigo from "./distro/dist/src/distro.js";
import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import {register} from "module";

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

const awsLambdaInstrumentation = new AwsLambdaInstrumentation({});

// Override _onRequire on the instance to fix non-configurable exports before patching.
// We can't override init() because it's already called during construction (via enable()).
// _onRequire is called lazily when modules are required, so this override takes effect
// before the Lambda runtime loads the handler.
const originalOnRequire = awsLambdaInstrumentation._onRequire;
awsLambdaInstrumentation._onRequire = function(module, exports, name, basedir) {
    return originalOnRequire.call(this, module, makeExportsConfigurable(exports), name, basedir);
};

const tracerProvider = (await lumigo.init).tracerProvider;

registerInstrumentations({
    instrumentations: [
        awsLambdaInstrumentation
    ],
    tracerProvider
});

register('import-in-the-middle/hook.mjs', import.meta.url);
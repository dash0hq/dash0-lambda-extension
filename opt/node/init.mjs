console.log('[init.mjs] file top');
import * as dash0 from "./distro/dist/src/distro.js";

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


    const tracerProvider = (await dash0.init).tracerProvider;

    // registerInstrumentations({
    //     instrumentations: [
    //         awsLambdaInstrumentation
    //     ],
    //     tracerProvider
    // });

    console.log('[init.mjs] after registerInstrumentations');


} catch (err) {
    console.error('Error initializing Dash0 tracer:', err);
}
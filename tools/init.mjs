import {register} from "module";
import {Hook} from 'import-in-the-middle';
import { context, trace, SpanKind } from '@opentelemetry/api';
import path from 'path';

// CRITICAL: Register import-in-the-middle FIRST, before any other imports
// This ensures the hook can intercept all module loads, including the handler
register('import-in-the-middle/hook.mjs', import.meta.url);

// Set up ESM hook SYNCHRONOUSLY before any async operations
const taskRoot = process.env.LAMBDA_TASK_ROOT;
const handlerDef = process.env._HANDLER;

let tracerProviderPromise = null;

if (taskRoot && handlerDef) {
    const handler = path.basename(handlerDef);
    const [moduleName, functionName] = handler.split('.', 2);
    const handlerPath = path.resolve(taskRoot, `${moduleName}.mjs`);

    console.log(`Setting up ESM hook for handler module: ${moduleName}, function: ${functionName}`);
    console.log(`Handler path: ${handlerPath}`);

    // Hook ALL modules to see what's being loaded
    Hook(null, (exported, name, baseDir) => {
        // Check if this is our handler module by multiple criteria
        const isHandlerModule =
            name === moduleName ||
            name === handlerPath ||
            name.endsWith(`${moduleName}.mjs`) ||
            name.includes('/var/task/index');

        if (isHandlerModule && exported && typeof exported[functionName] === 'function') {
            console.log(`✓ ESM Hook intercepted handler module: ${name}`);

            const originalHandler = exported[functionName];

            // Wrap using AwsLambdaInstrumentation's internal logic
            exported[functionName] = async function wrappedHandler(event, lambdaContext, callback) {
                console.log('✓ ESM handler invoked, applying AwsLambdaInstrumentation wrapper');

                // Wait for instrumentation to be ready
                const { instrumentation } = await tracerProviderPromise;

                // Get the wrapper function from AwsLambdaInstrumentation
                // _getHandler returns a function that wraps the original handler
                const lambdaStartTime = Date.now() - Math.floor(1000 * process.uptime());
                const wrapperFactory = instrumentation._getHandler(lambdaStartTime);
                const wrappedOriginal = wrapperFactory(originalHandler);

                // Call the wrapped handler
                return wrappedOriginal.call(this, event, lambdaContext, callback);
            };
        }

        return exported;
    });
}

// Now do async initialization
import * as lumigo from "@lumigo/opentelemetry";
import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';
import { registerInstrumentations } from '@opentelemetry/instrumentation';

console.log("Lumigo OpenTelemetry initialized in Node.js Lambda function...");

// Create instrumentation instance
const awsLambdaInstrumentation = new AwsLambdaInstrumentation({
    requestHook: (span, { event, context }) => {
        span.setAttribute('faas.name', context.functionName);
        span.setAttribute('faas.nothing', context.functionName);
        console.log("in requesthool.");
    },
    responseHook: (span, { err, res }) => {
        if (err instanceof Error) span.setAttribute('faas.error', err.message);
        if (res) span.setAttribute('faas.res', res);
    }
});

// For CJS handlers only
registerInstrumentations({
    instrumentations: [
        awsLambdaInstrumentation
    ],
});

const { tracerProvider } = await lumigo.init;
awsLambdaInstrumentation.setTracerProvider(tracerProvider);

// Store instrumentation instance for the ESM handler wrapper
tracerProviderPromise = Promise.resolve({ tracerProvider, instrumentation: awsLambdaInstrumentation });

// registerInstrumentations({
//     instrumentations: [
//         awsLambdaInstrumentation
//     ],
//     tracerProvider
// });


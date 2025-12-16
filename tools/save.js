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
        // Log every module that loads
        console.log(`Hook saw module: ${name}`);

        // Check if this is our handler module by multiple criteria
        const isHandlerModule =
            name === moduleName ||
            name === handlerPath ||
            name.endsWith(`${moduleName}.mjs`) ||
            name.includes('/var/task/index');

        if (isHandlerModule) {
            console.log(`✓ MATCHED handler module: ${name}`);

            if (exported && typeof exported[functionName] === 'function') {
                console.log(`✓ Found handler function: ${functionName}, wrapping it...`);

                const originalHandler = exported[functionName];

                // Create instrumented wrapper
                exported[functionName] = async function instrumentedHandler(event, lambdaContext) {
                    console.log('✓ Instrumented ESM handler invoked');

                    // Wait for tracer provider to be ready
                    const tracerProvider = await tracerProviderPromise;
                    const tracer = tracerProvider.getTracer('aws-lambda-instrumentation');

                    const span = tracer.startSpan(lambdaContext.functionName, {
                        kind: SpanKind.SERVER,
                        attributes: {
                            'faas.execution': lambdaContext.requestId,
                            'faas.id': lambdaContext.invokedFunctionArn,
                            'cloud.account.id': lambdaContext.invokedFunctionArn.split(':')[4],
                            'faas.name': lambdaContext.functionName,
                            'faas.nothing': lambdaContext.functionName,
                        }
                    });

                    console.log('in requesthool.');

                    return context.with(trace.setSpan(context.active(), span), async () => {
                        try {
                            const result = await originalHandler.call(this, event, lambdaContext);
                            if (result) span.setAttribute('faas.res', JSON.stringify(result));
                            span.setStatus({ code: 1 }); // OK
                            return result;
                        } catch (err) {
                            span.setAttribute('faas.error', err.message);
                            span.setStatus({ code: 2, message: err.message }); // ERROR
                            throw err;
                        } finally {
                            span.end();
                        }
                    });
                };
            } else {
                console.log(`✗ Module matched but no function '${functionName}' found. Exports:`, Object.keys(exported || {}));
            }
        }

        return exported;
    });
}

// Now do async initialization
import * as lumigo from "@lumigo/opentelemetry";
import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';
import { registerInstrumentations } from '@opentelemetry/instrumentation';

console.log("Lumigo OpenTelemetry initialized in Node.js Lambda function...");

// For CJS handlers
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
registerInstrumentations({
    instrumentations: [
        awsLambdaInstrumentation
    ],
});

const { tracerProvider } = await lumigo.init;
awsLambdaInstrumentation.setTracerProvider(tracerProvider);

// Store tracer provider for the ESM handler wrapper
tracerProviderPromise = Promise.resolve(tracerProvider);

// registerInstrumentations({
//     instrumentations: [
//         awsLambdaInstrumentation
//     ],
//     tracerProvider
// });


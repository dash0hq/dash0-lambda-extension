console.log("init.mjs line 1");

import * as lumigo from "@lumigo/opentelemetry";
console.log("init.mjs line 4");

import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';
console.log("init.mjs line 7");

import { registerInstrumentations } from '@opentelemetry/instrumentation';
console.log("init.mjs line 10");

import {register} from "module";
console.log("init.mjs line 13");


console.log("Lumigo OpenTelemetry initialized in Node.js Lambda function.....");

const awsLambdaInstrumentation = new AwsLambdaInstrumentation({});
console.log("init.mjs line 19");
const tracerProvider = (await lumigo.init).tracerProvider;
console.log("init.mjs line 21");

registerInstrumentations({
    instrumentations: [
        awsLambdaInstrumentation
    ],
    tracerProvider
});
console.log("init.mjs line 29");

register('import-in-the-middle/hook.mjs', import.meta.url);
console.log("init.mjs line 32");

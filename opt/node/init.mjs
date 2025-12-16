import * as lumigo from "@lumigo/opentelemetry";
import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import {register} from "module";


console.log("Lumigo OpenTelemetry initialized in Node.js Lambda function.....");

const awsLambdaInstrumentation = new AwsLambdaInstrumentation({});
const tracerProvider = (await lumigo.init).tracerProvider;

registerInstrumentations({
    instrumentations: [
        awsLambdaInstrumentation
    ],
    tracerProvider
});

register('import-in-the-middle/hook.mjs', import.meta.url);
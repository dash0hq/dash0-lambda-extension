import * as lumigo from "./distro/dist/src/distro.js";
import { AwsLambdaInstrumentation } from '@opentelemetry/instrumentation-aws-lambda';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import {register} from "module";


const awsLambdaInstrumentation = new AwsLambdaInstrumentation({});
const tracerProvider = (await lumigo.init).tracerProvider;

registerInstrumentations({
    instrumentations: [
        awsLambdaInstrumentation
    ],
    tracerProvider
});

register('import-in-the-middle/hook.mjs', import.meta.url);
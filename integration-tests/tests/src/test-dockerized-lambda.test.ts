import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import {
    checkLogs,
    checkSpanAttributesFromReport,
    compareJsonStrings,
    getAttributesMap,
    getRequestPayload,
    invokeFunction,
} from "./utils";


const verifyDockerizedInvocation = async (functionName: string, runtime: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    let traceId: string | undefined = undefined;
    let parentSpanId: string | undefined = undefined;
    let span = undefined
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch spans for invocation ID ${invocationId}`);
        try {
            const spanResponse = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify(getRequestPayload(invocationId)),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toEqual(1);
            const scopeNameMap = {
                "python": "opentelemetry.instrumentation.aws_lambda",
                "node": "@opentelemetry/instrumentation-aws-lambda",
                "java": "io.opentelemetry.aws-lambda-events-2.2"
            }
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual(scopeNameMap[runtime as keyof typeof scopeNameMap]);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);
            // check span attributes
            span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            expect(spanAttributes['dash0.faas.event'].stringValue).toEqual('{"parameter1":"right"}');
            const returnValue = runtime === 'java' ? '"Hello World from Java Lambda!"' : '{"statusCode":200,"body":"{\\"message\\":\\"Success\\"}"}';
            compareJsonStrings(spanAttributes['dash0.faas.return_value'].stringValue, returnValue);
            expect(spanAttributes['dash0.faas.init_duration'].doubleValue).toBeGreaterThan(0);

            const resourceAttributes = getAttributesMap(spanPayload?.resourceSpans[0].resource.attributes);
            expect(resourceAttributes['service.name'].stringValue).toEqual(functionName);

            traceId = span.traceId;
            parentSpanId = span.spanId;
            break;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }

    const logsToBeChecked = [
        'START RequestId: ',
        'END RequestId: ',
    ]
    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: parentSpanId!,
        success: true,
        logsToBeChecked
    });
}

describe.concurrent('Dockerized Lambda invocation', () => {
    const runtimes = ['python', 'node', 'java'];
    const architectures = ['x86_64', 'arm64'];

    for (const runtime of runtimes) {
        for (const architecture of architectures) {
            const functionName = `dockerized-${runtime}-${architecture}`;
            it(functionName, async () => {
                await verifyDockerizedInvocation(functionName, runtime);
            }, 120_000);
        }
    }
});

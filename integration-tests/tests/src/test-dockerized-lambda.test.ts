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


const verifyDockerizedInvocation = async (functionName: string, invocationEnd: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, false);

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
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual("opentelemetry.instrumentation.aws_lambda");
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);
            // check span attributes
            span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            expect(spanAttributes['faas.event'].stringValue).toEqual('{"parameter1":"right"}');
            compareJsonStrings(spanAttributes['faas.return_value'].stringValue, '{"statusCode":200,"body":"{\\"message\\":\\"Success\\"}"}');
            expect(spanAttributes['faas.init_duration'].doubleValue).toBeGreaterThan(0);

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
        'event payload: ',
        'END RequestId: ',
    ]
    if (!invocationEnd) {
        logsToBeChecked.push('REPORT RequestId: ');
    }
    const reportLog = await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: parentSpanId!,
        success: true,
        logsToBeChecked
    });
    if (!invocationEnd) {
        checkSpanAttributesFromReport(reportLog, span);
    }
}

describe.concurrent('Dockerized Lambda invocation', () => {
    it('dockerized-python', async () => {
        await verifyDockerizedInvocation('dockerized-python', true);
    }, 120_000);
});

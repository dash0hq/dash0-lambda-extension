import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {
    checkException,
    checkHttpSpan,
    checkLogs,
    checkResourceAttributes,
    getAttributesMap,
    getRequestPayload,
    invokeFunction,
    runAllTests
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    let traceId: string | undefined = undefined;
    let parentSpanId: string | undefined = undefined;
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
            const span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            expect(spanAttributes['dash0.faas.event'].stringValue).toEqual('{"parameter1":"right"}');
            expect(spanAttributes['faas.init_duration'].doubleValue).toBeGreaterThan(0);
            checkResourceAttributes(spanPayload.resourceSpans[0].resource.attributes, functionName);
            checkException(span, 'timeout');
            traceId = spanPayload.resourceSpans[0].scopeSpans[0].spans[0].traceId;
            parentSpanId = spanPayload.resourceSpans[0].scopeSpans[0].spans[0].spanId;
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
        "Handler invoked with event:",
        'END RequestId: ',
    ];
    if (!invocationEnd) {
        logsToBeChecked.push("REPORT RequestId: ", "Status: timeout");
    }
    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: parentSpanId!,
        success: false,
        logsToBeChecked,
    });
}

describe.concurrent('Lambda invocations with timeout', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    runAllTests('timeout', runtimes, verifySuccessInvocation);
});

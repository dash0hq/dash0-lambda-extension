import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import {checkLogs, findHandlerSpan, getAttributesMap, getRequestPayload, invokeFunction, LogToCheck, RESOURCE_PREFIX} from "./utils";

const verifyManualInstrumentation = async (functionName: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    let traceId: string | undefined = undefined;
    let parentSpanId: string | undefined = undefined;
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch spans for function ${functionName}`);
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
            expect(spanPayload?.resourceSpans?.length).toBeGreaterThanOrEqual(1);

            const { span, resource } = findHandlerSpan(spanPayload, "@opentelemetry/instrumentation-aws-lambda");
            const resourceAttributes = getAttributesMap(resource.attributes);
            expect(resourceAttributes['service.name'].stringValue).toEqual(functionName);
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);

            traceId = span.traceId;
            parentSpanId = span.spanId;
            return;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: '[tracing] forceFlush complete' },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200 } }), isJson: true },
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

describe('Manual instrumentation Lambda', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}manual-instrumentation-${runtime}`
        it(
            `invokes ${functionName} and receives trace`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyManualInstrumentation(functionName);
            },
            120_000
        );
    }
});

import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import {
    checkLogs,
    compareJsonStrings,
    getAttributesMap,
    getRequestPayload, LogToCheck,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";

const verifyCjsSuccess = async (functionName: string) => {
    const invocationPayload = JSON.stringify({ parameter1: 'right' });
    const invocationId = await invokeFunction(functionName, true, false, invocationPayload);

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
            expect(spanPayload?.resourceSpans?.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual(
                "@opentelemetry/instrumentation-aws-lambda"
            );
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);

            const span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            compareJsonStrings(
                spanAttributes['dash0.faas.return_value'].stringValue,
                '{"statusCode":200,"body":"{\\"message\\":\\"Success\\"}"}'
            );

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

    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: parentSpanId!,
        success: true,
        logsToBeChecked: [
            { message: 'START RequestId: ' },
            { message: 'Handler invoked with event:' },
            { message: 'END RequestId: ' },
        ],
    });
};

describe.concurrent('CJS-bundled Lambda invocation', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}cjs-success-${runtime}`;
        it(
            `invokes ${functionName} successfully`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyCjsSuccess(functionName);
            },
            120_000
        );
    }
});

import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import {compareJsonStrings, getAttributesMap, getRequestPayload, invokeFunction} from "./utils";

const FUNCTION_NAME = 'manual-instrumentation-node';

const verifyManualInstrumentation = async () => {
    const invocationId = await invokeFunction(FUNCTION_NAME, true, false);

    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch spans for function ${FUNCTION_NAME}`);
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
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toEqual(1);
            const expectedScopeName = "@opentelemetry/instrumentation-aws-lambda";
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual(expectedScopeName);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);
            const resourceAttributes = getAttributesMap(spanPayload?.resourceSpans[0].resource.attributes);
            expect(resourceAttributes['service.name'].stringValue).toEqual(FUNCTION_NAME);
            // check span attributes
            const span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            compareJsonStrings(spanAttributes['faas.event'].stringValue, '{"parameter1":"right"}');
            compareJsonStrings(spanAttributes['faas.return_value'].stringValue, '{"statusCode":200,"body":"{\\"message\\":\\"Success\\"}"}');

            return;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
}

describe('Manual instrumentation Lambda', () => {
    it(
        `invokes ${FUNCTION_NAME} and receives trace`,
        async () => {
            console.log(`Starting test for ${FUNCTION_NAME}`, new Date().toISOString());
            await verifyManualInstrumentation();
        },
        120_000
    );
});

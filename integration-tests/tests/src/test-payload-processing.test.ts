import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import { getAttributesMap, getRequestPayload, invokeFunction } from "./utils";

describe.concurrent('Payload processing', () => {
    it('truncates large event payload (~30KB)', async () => {
        const functionName = 'python3-14-success-true-invocation-end-true-arm64';

        // Build a ~30KB JSON payload
        const largeValue = 'x'.repeat(30_000);
        const eventPayload = JSON.stringify({ abc: "xyz", data: largeValue });

        const invocationId = await invokeFunction(functionName, true, false, eventPayload);

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

                const span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
                const spanAttributes = getAttributesMap(span.attributes);

                expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);

                // The event attribute should exist and be truncated below the 20KB default limit
                const eventAttr = spanAttributes['dash0.faas.event'].stringValue;
                expect(eventAttr).toEqual("{\"abc\":\"xyz\",\"data\":\"[truncated]\"}")

                break;
            } catch (error) {
                console.error(`Error fetching spans on attempt ${attempt}:`, error);
                if (attempt === MAX_ATTEMPTS) {
                    throw error;
                }
            }
        }
    }, 120_000);
});

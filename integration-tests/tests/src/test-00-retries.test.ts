import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { PYTHON_RUNTIMES } from '../../runtimes';
import { DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN } from './config';
import { RESOURCE_PREFIX } from './utils';
import { invokeProducerAndGetLeafSpan } from './utils-tracing-scenarios';

const pythonRuntimes = PYTHON_RUNTIMES.filter(r => r !== 'python3-10');

const EXPECTED_CONSUMER_INVOCATIONS = 2;

const fetchAndVerifyConsumerRetries = async (
    consumerFunctionName: string,
    expectedScopeName: string,
    producerTraceId: string,
    leafSpanId: string,
) => {
    // Retries take time (~1min + ~2min), use more attempts with longer delay
    const maxAttempts = 30;
    const retryDelay = 10_000;

    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
        await delay(retryDelay);
        console.log(`Attempt ${attempt} to fetch consumer retry spans for ${consumerFunctionName}`);
        try {
            const now = Date.now();
            const spanResponse = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify({
                    filter: [{ operator: 'contains', key: 'service.name', value: consumerFunctionName }],
                    timeRange: {
                        from: new Date(now - 10 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: { mode: 'adaptive' },
                    dataset: DASH0_LAMBDA_TESTS_DATASET,
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            // Collect all consumer root spans: from dash0.lambda-extension scope,
            // matching the producer traceId and with parentSpanId = leafSpanId
            const consumerRootSpans: any[] = [];
            for (const rs of spanPayload.resourceSpans) {
                for (const ss of rs.scopeSpans) {
                    if (ss.scope?.name === 'dash0.lambda-extension') {
                        for (const span of ss.spans) {
                            if (span.traceId === producerTraceId && span.parentSpanId === leafSpanId) {
                                consumerRootSpans.push(span);
                            }
                        }
                    }
                }
            }

            console.log(`Found ${consumerRootSpans.length} consumer root spans (expecting ${EXPECTED_CONSUMER_INVOCATIONS})`);
            if (consumerRootSpans.length < EXPECTED_CONSUMER_INVOCATIONS) {
                if (attempt === maxAttempts) {
                    throw new Error(
                        `Expected ${EXPECTED_CONSUMER_INVOCATIONS} consumer root spans but found ${consumerRootSpans.length}`,
                    );
                }
                continue;
            }

            expect(consumerRootSpans.length).toBeGreaterThanOrEqual(EXPECTED_CONSUMER_INVOCATIONS);

            // Each root span should have a unique spanId
            const rootSpanIds = consumerRootSpans.map(s => s.spanId);
            expect(new Set(rootSpanIds).size).toBeGreaterThanOrEqual(EXPECTED_CONSUMER_INVOCATIONS);
            console.log(`Consumer root span IDs: ${rootSpanIds.join(', ')}`);

            // All root spans should point to the same parent (leaf client span from producer)
            for (const rootSpan of consumerRootSpans) {
                expect(rootSpan.parentSpanId).toEqual(leafSpanId);
                expect(rootSpan.traceId).toEqual(producerTraceId);
            }

            // Each root span should have a handler span child
            const allConsumerSpans: any[] = [];
            for (const rs of spanPayload.resourceSpans) {
                for (const ss of rs.scopeSpans) {
                    if (ss.scope.name === expectedScopeName) {
                        for (const span of ss.spans) {
                            if (span.traceId === producerTraceId) {
                                allConsumerSpans.push(span);
                            }
                        }
                    }
                }
            }

            for (const rootSpan of consumerRootSpans) {
                const handlerSpan = allConsumerSpans.find(s => s.parentSpanId === rootSpan.spanId);
                expect(handlerSpan, `Handler span not found for root span ${rootSpan.spanId}`).toBeDefined();
                console.log(`Root ${rootSpan.spanId} -> Handler ${handlerSpan.spanId}`);
            }

            return;
        } catch (error) {
            console.error(`Error fetching consumer retry spans on attempt ${attempt}:`, error);
            if (attempt === maxAttempts) throw error;
        }
    }
};

const verifyRetryScenario = async (
    producerFunctionName: string,
    consumerFunctionName: string,
) => {
    const { producerTraceId, leafSpanId, expectedConsumerScopeName } =
        await invokeProducerAndGetLeafSpan(producerFunctionName, consumerFunctionName);

    await fetchAndVerifyConsumerRetries(
        consumerFunctionName, expectedConsumerScopeName, producerTraceId, leafSpanId,
    );
};

describe.concurrent('Retry Scenarios', () => {
    for (const runtime of pythonRuntimes) {
        const producerFunctionName = `${RESOURCE_PREFIX}tracing-eventbridge-producer-error-${runtime}`;
        const consumerFunctionName = `${RESOURCE_PREFIX}tracing-eventbridge-consumer-error-${runtime}`;

        it(
            `verifies eventbridge retry tracing for ${runtime}`,
            async () => {
                console.log(`Starting retry test for ${runtime}`, new Date().toISOString());
                await verifyRetryScenario(producerFunctionName, consumerFunctionName);
            },
            1_200_000,
        );
    }
});

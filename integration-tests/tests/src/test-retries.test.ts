import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from './config';
import { getRequestPayload, invokeFunction, RESOURCE_PREFIX } from './utils';

const pythonRuntimes = ['python3-11', 'python3-12', 'python3-13', 'python3-14'];

const EXPECTED_CONSUMER_INVOCATIONS = 3; // 1 original + 2 retries (Lambda default)

const getLambdaScopeName = (functionName: string) =>
    functionName.includes('python')
        ? 'opentelemetry.instrumentation.aws_lambda'
        : '@opentelemetry/instrumentation-aws-lambda';

const fetchProducerSpans = async (
    producerInvocationId: string,
    expectedScopeName: string,
): Promise<{ traceId: string; handlerSpanId: string; rootSpanId: string }> => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch producer spans for invocation ID ${producerInvocationId}`);
        try {
            const spanResponse = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify(getRequestPayload(producerInvocationId)),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            let handlerSpan: any = null;
            for (const rs of spanPayload.resourceSpans) {
                for (const ss of rs.scopeSpans) {
                    if (ss.scope.name === expectedScopeName) {
                        expect(ss.spans.length).toEqual(1);
                        handlerSpan = ss.spans[0];
                    }
                }
            }
            expect(handlerSpan, `Producer handler span not found in scope ${expectedScopeName}`).not.toBeNull();

            let rootSpan: any = null;
            for (const rs of spanPayload.resourceSpans) {
                for (const ss of rs.scopeSpans) {
                    if (ss.scope?.name === 'dash0.lambda-extension') {
                        for (const span of ss.spans) {
                            if (span.spanId === handlerSpan.parentSpanId) {
                                rootSpan = span;
                            }
                        }
                    }
                }
            }
            expect(rootSpan, 'Producer root span not found').not.toBeNull();
            expect(rootSpan.traceId).toEqual(handlerSpan.traceId);

            console.log(`Producer traceId: ${handlerSpan.traceId}, handlerSpanId: ${handlerSpan.spanId}, rootSpanId: ${rootSpan.spanId}`);
            return { traceId: handlerSpan.traceId, handlerSpanId: handlerSpan.spanId, rootSpanId: rootSpan.spanId };
        } catch (error) {
            console.error(`Error fetching producer spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) throw error;
        }
    }
    throw new Error('Failed to fetch producer spans');
};

const fetchLeafClientSpanId = async (
    producerFunctionName: string,
    producerTraceId: string,
    producerSpanId: string,
): Promise<string> => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch client spans for ${producerFunctionName}`);
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
                    filter: [{ operator: 'contains', key: 'service.name', value: producerFunctionName }],
                    timeRange: {
                        from: new Date(now - 5 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: { mode: 'adaptive' },
                    dataset: DASH0_LAMBDA_TESTS_DATASET,
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            const allSpans: any[] = [];
            for (const resourceSpan of spanPayload.resourceSpans) {
                for (const scopeSpan of resourceSpan.scopeSpans) {
                    for (const span of scopeSpan.spans) {
                        if (span.traceId === producerTraceId) {
                            allSpans.push(span);
                        }
                    }
                }
            }

            let currentSpanId = producerSpanId;
            let depth = 0;
            while (true) {
                const child = allSpans.find(s => s.parentSpanId === currentSpanId);
                if (!child) break;
                depth++;
                currentSpanId = child.spanId;
                console.log(`  depth ${depth}: spanId=${child.spanId}, child of ${child.parentSpanId}`);
            }

            expect(depth).toBeGreaterThanOrEqual(1);
            console.log(`Leaf client span found: spanId=${currentSpanId} (depth=${depth})`);
            return currentSpanId;
        } catch (error) {
            console.error(`Error fetching client spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) throw error;
        }
    }
    throw new Error('Failed to fetch leaf client span');
};

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

            expect(consumerRootSpans.length).toEqual(EXPECTED_CONSUMER_INVOCATIONS);

            // Each root span should have a unique spanId
            const rootSpanIds = consumerRootSpans.map(s => s.spanId);
            expect(new Set(rootSpanIds).size).toEqual(EXPECTED_CONSUMER_INVOCATIONS);
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
    const producerInvocationId = await invokeFunction(producerFunctionName, true, false);
    console.log(`Producer invocation ID: ${producerInvocationId}`);

    // Invoke producer again to flush its supplementary spans
    await delay(8000);
    await invokeFunction(producerFunctionName, true, false);

    const expectedProducerScopeName = getLambdaScopeName(producerFunctionName);
    const expectedConsumerScopeName = getLambdaScopeName(consumerFunctionName);

    const { traceId: producerTraceId, handlerSpanId: producerHandlerSpanId } =
        await fetchProducerSpans(producerInvocationId, expectedProducerScopeName);

    const leafSpanId =
        await fetchLeafClientSpanId(producerFunctionName, producerTraceId, producerHandlerSpanId);

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
            600_000,
        );
    }
});

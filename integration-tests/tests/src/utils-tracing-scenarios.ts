import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { expect } from 'vitest';
import { DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from './config';
import { getRequestPayload, invokeFunction } from './utils';

export const getLambdaScopeName = (functionName: string) =>
    functionName.includes('python')
        ? 'opentelemetry.instrumentation.aws_lambda'
        : '@opentelemetry/instrumentation-aws-lambda';

export const fetchProducerSpans = async (
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

            // Find handler span from lambda instrumentation scope
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

            // Find root span from dash0.lambda-extension scope (its spanId should match handler's parentSpanId)
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
            expect(rootSpan, 'Producer root span not found in dash0.lambda-extension scope').not.toBeNull();
            expect(rootSpan.traceId).toEqual(handlerSpan.traceId);

            console.log(`Producer traceId: ${handlerSpan.traceId}, handlerSpanId: ${handlerSpan.spanId}, rootSpanId: ${rootSpan.spanId}`);
            return { traceId: handlerSpan.traceId, handlerSpanId: handlerSpan.spanId, rootSpanId: rootSpan.spanId };
        } catch (error) {
            console.error(`Error fetching producer spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    throw new Error('Failed to fetch producer spans');
};

export const fetchLeafClientSpanId = async (
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
                    filter: [
                        {
                            operator: 'contains',
                            key: 'service.name',
                            value: producerFunctionName,
                        },
                    ],
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

            // Collect all spans from the producer's service into a flat list
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

            // Walk the chain from the producer span to the deepest descendant
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
            return currentSpanId!;
        } catch (error) {
            console.error(`Error fetching client spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    throw new Error('Failed to fetch leaf client span');
};

export const invokeProducerAndGetLeafSpan = async (
    producerFunctionName: string,
    consumerFunctionName: string,
): Promise<{ producerTraceId: string; leafSpanId: string; expectedConsumerScopeName: string }> => {
    const producerInvocationId = await invokeFunction(producerFunctionName, true, false);
    console.log(`Producer invocation ID: ${producerInvocationId}`);

    // Invoke producer again to flush its supplementary spans (root span arrives on next invocation)
    await delay(8000);
    await invokeFunction(producerFunctionName, true, false);

    const expectedProducerScopeName = getLambdaScopeName(producerFunctionName);
    const expectedConsumerScopeName = getLambdaScopeName(consumerFunctionName);

    const { traceId: producerTraceId, handlerSpanId: producerHandlerSpanId } =
        await fetchProducerSpans(producerInvocationId, expectedProducerScopeName);

    const leafSpanId =
        await fetchLeafClientSpanId(producerFunctionName, producerTraceId, producerHandlerSpanId);

    return { producerTraceId, leafSpanId, expectedConsumerScopeName };
};

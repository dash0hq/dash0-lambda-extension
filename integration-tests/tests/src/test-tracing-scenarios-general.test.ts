import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from './config';
import { getAttributesMap, getRequestPayload, invokeFunction, RESOURCE_PREFIX } from './utils';

const pythonRuntimes = ['python3-11', 'python3-12', 'python3-13', 'python3-14'];
const nodeRuntimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];

const scenarios = [
    { name: 'eventbridge', producerPrefix: `${RESOURCE_PREFIX}tracing-eventbridge-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-eventbridge-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'apigateway', producerPrefix: `${RESOURCE_PREFIX}tracing-apigateway-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-apigateway-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 's3', producerPrefix: `${RESOURCE_PREFIX}tracing-s3-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-s3-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'lambda', producerPrefix: `${RESOURCE_PREFIX}tracing-lambda-invoker`, consumerPrefix: `${RESOURCE_PREFIX}tracing-lambda-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
] as const;

const getLambdaScopeName = (functionName: string) =>
    functionName.includes('python') ?
        'opentelemetry.instrumentation.aws_lambda' :
        '@opentelemetry/instrumentation-aws-lambda';

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
                    dataset: DASH0_LAMBDA_TESTS_DATASET
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

const fetchAndVerifyConsumerSpans = async (
    consumerFunctionName: string,
    expectedScopeName: string,
    producerTraceId: string,
    leafSpanId: string,
    scenarioName: string,
) => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch consumer spans for ${consumerFunctionName}`);
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
                            value: consumerFunctionName,
                        },
                    ],
                    timeRange: {
                        from: new Date(now - 5 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: { mode: 'adaptive' },
                    dataset: DASH0_LAMBDA_TESTS_DATASET
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            // Find consumer root span (from dash0.lambda-extension scope, child of the leaf client span)
            let consumerRootSpan: any = null;
            for (const resourceSpan of spanPayload.resourceSpans) {
                for (const scopeSpan of resourceSpan.scopeSpans) {
                    if (scopeSpan.scope?.name === 'dash0.lambda-extension') {
                        for (const span of scopeSpan.spans) {
                            if (span.traceId === producerTraceId && span.parentSpanId === leafSpanId) {
                                consumerRootSpan = span;
                                break;
                            }
                        }
                    }
                    if (consumerRootSpan) break;
                }
                if (consumerRootSpan) break;
            }
            expect(consumerRootSpan, `Consumer root span not found: ${producerTraceId}`).not.toBeNull();
            console.log(`Consumer root span found: spanId=${consumerRootSpan!.spanId}, parentSpanId=${consumerRootSpan!.parentSpanId}`);

            // Find consumer handler span (from lambda scope, child of consumer root span)
            let consumerHandlerSpan: any = null;
            for (const resourceSpan of spanPayload.resourceSpans) {
                for (const scopeSpan of resourceSpan.scopeSpans) {
                    if (scopeSpan.scope.name === expectedScopeName) {
                        for (const span of scopeSpan.spans) {
                            if (span.traceId === producerTraceId && span.parentSpanId === consumerRootSpan!.spanId) {
                                consumerHandlerSpan = span;
                                break;
                            }
                        }
                    }
                    if (consumerHandlerSpan) break;
                }
                if (consumerHandlerSpan) break;
            }
            expect(consumerHandlerSpan, 'Consumer handler span not found').not.toBeNull();
            console.log(`Consumer handler span found: spanId=${consumerHandlerSpan!.spanId}, parentSpanId=${consumerHandlerSpan!.parentSpanId}`);
            console.log(`Trace chain verified: leaf(${leafSpanId}) -> consumer root(${consumerRootSpan!.spanId}) -> consumer handler(${consumerHandlerSpan!.spanId})`);

            if (scenarioName === 'eventbridge') {
                const consumerAttrs = getAttributesMap(consumerHandlerSpan!.attributes);
                expect(consumerAttrs['faas.trigger']).toBeDefined();
                expect(consumerAttrs['dash0.faas.event_bridge_source']).toBeDefined();
                expect(consumerAttrs['dash0.faas.event_bridge_detail_type']).toBeDefined();
                console.log(`EventBridge attributes: faas.trigger=${JSON.stringify(consumerAttrs['faas.trigger'])}, source=${JSON.stringify(consumerAttrs['dash0.faas.event_bridge_source'])}, detail_type=${JSON.stringify(consumerAttrs['dash0.faas.event_bridge_detail_type'])}`);
            }

            return;
        } catch (error) {
            console.error(`Error fetching consumer spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
};

const verifyTracingScenario = async (
    producerFunctionName: string,
    consumerFunctionName: string,
    scenarioName: string,
) => {
    // invocationEnd=false triggers a second invocation to flush supplementary spans (including root span)
    const producerInvocationId = await invokeFunction(producerFunctionName, true, false);
    console.log(`Producer invocation ID: ${producerInvocationId}`);

    // Invoke producer to flush its supplementary spans (root span arrives on next invocation)
    await delay(8000);
    await invokeFunction(producerFunctionName, true, false);

    const expectedProducerScopeName = getLambdaScopeName(producerFunctionName);
    const expectedConsumerScopeName = getLambdaScopeName(consumerFunctionName);

    // Fetch producer handler span and root span, verify root -> handler link
    const { traceId: producerTraceId, handlerSpanId: producerHandlerSpanId } =
        await fetchProducerSpans(producerInvocationId, expectedProducerScopeName);

    // Fetch leaf client span (deepest descendant of producer handler)
    const leafSpanId =
        await fetchLeafClientSpanId(producerFunctionName, producerTraceId, producerHandlerSpanId);

    // Verify consumer: root span (child of leaf) -> handler (child of consumer root)
    await fetchAndVerifyConsumerSpans(
        consumerFunctionName, expectedConsumerScopeName, producerTraceId, leafSpanId, scenarioName,
    );
};

describe.concurrent('Tracing Scenarios', () => {
    for (const scenario of scenarios) {
        for (const runtime of scenario.runtimes) {
            const producerFunctionName = `${scenario.producerPrefix}-${runtime}`;
            const consumerFunctionName = `${scenario.consumerPrefix}-${runtime}`;

            it(
                `verifies ${scenario.name} tracing for ${runtime}`,
                async () => {
                    console.log(`Starting test for ${scenario.name} scenario with ${runtime}`, new Date().toISOString());
                    await verifyTracingScenario(producerFunctionName, consumerFunctionName, scenario.name);
                },
                180_000
            );
        }
    }
});

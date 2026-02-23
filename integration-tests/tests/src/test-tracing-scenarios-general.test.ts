import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from './config';
import { getRequestPayload, invokeFunction, RESOURCE_PREFIX } from './utils';

const pythonRuntimes = ['python3-11', 'python3-12', 'python3-13', 'python3-14'];
const nodeRuntimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];

const scenarios = [
    { name: 'eventbridge', producerPrefix: `${RESOURCE_PREFIX}tracing-eventbridge-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-eventbridge-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'apigateway', producerPrefix: `${RESOURCE_PREFIX}tracing-apigateway-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-apigateway-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
] as const;

const getLambdaScopeName = (functionName: string) =>
    functionName.includes('python') ?
        'opentelemetry.instrumentation.aws_lambda' :
        '@opentelemetry/instrumentation-aws-lambda';

const verifyTracingScenario = async (
    producerFunctionName: string,
    consumerFunctionName: string,
) => {
    // Step 1: Invoke the producer lambda
    const producerInvocationId = await invokeFunction(producerFunctionName, true, false);
    console.log(`Producer invocation ID: ${producerInvocationId}`);

    const expectedScopeName = getLambdaScopeName(producerFunctionName);

    // Step 2: Fetch the producer (lambda) span by invocation ID
    let producerTraceId: string | undefined;
    let producerSpanId: string | undefined;

    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch producer span for invocation ID ${producerInvocationId}`);
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
            expect(spanPayload?.resourceSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toBeGreaterThanOrEqual(1);

            const lambdaScopeSpan = spanPayload.resourceSpans[0].scopeSpans.find(
                (ss: any) => ss.scope.name === expectedScopeName
            );
            expect(lambdaScopeSpan).toBeDefined();
            expect(lambdaScopeSpan.spans.length).toEqual(1);

            const producerSpan = lambdaScopeSpan.spans[0];
            producerTraceId = producerSpan.traceId;
            producerSpanId = producerSpan.spanId;
            console.log(`Producer traceId: ${producerTraceId}, spanId: ${producerSpanId}`);
            break;
        } catch (error) {
            console.error(`Error fetching producer span on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }

    expect(producerTraceId).toBeDefined();
    expect(producerSpanId).toBeDefined();

    // Step 3: Fetch producer's child spans by service.name + time range, walk the
    //         hierarchy to find the deepest descendant (e.g. producer > eventbridge > http)
    let leafSpanId: string | undefined;

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
                console.log(`  depth ${depth}: spanId=${child.spanId}, scope child of ${child.parentSpanId}`);
            }

            expect(depth).toBeGreaterThanOrEqual(1);
            leafSpanId = currentSpanId;
            console.log(`Leaf client span found: spanId=${leafSpanId} (depth=${depth})`);
            break;
        } catch (error) {
            console.error(`Error fetching client spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }

    expect(leafSpanId).toBeDefined();

    // Step 4: Fetch consumer span and verify it is a child of the client span (grandchild of producer)
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch consumer span for ${consumerFunctionName}`);
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
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            // Find the consumer span that is a child of the client span (grandchild of producer)
            let consumerSpan: any = null;
            for (const resourceSpan of spanPayload.resourceSpans) {
                for (const scopeSpan of resourceSpan.scopeSpans) {
                    if (scopeSpan.scope.name === expectedScopeName) {
                        for (const span of scopeSpan.spans) {
                            if (span.traceId === producerTraceId && span.parentSpanId === leafSpanId) {
                                consumerSpan = span;
                                break;
                            }
                        }
                    }
                    if (consumerSpan) break;
                }
                if (consumerSpan) break;
            }

            expect(consumerSpan).not.toBeNull();
            console.log(`Consumer span found: traceId=${consumerSpan!.traceId}, spanId=${consumerSpan!.spanId}, parentSpanId=${consumerSpan!.parentSpanId}`);
            console.log(`Trace chain verified: producer(${producerSpanId}) -> client(${leafSpanId}) -> consumer(${consumerSpan!.spanId})`);
            break;
        } catch (error) {
            console.error(`Error fetching consumer span on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
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
                    await verifyTracingScenario(producerFunctionName, consumerFunctionName);
                },
                180_000
            );
        }
    }
});

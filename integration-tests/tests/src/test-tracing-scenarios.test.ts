import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from './config';
import { getAttributesMap, getRequestPayload, invokeFunction } from './utils';

const pythonRuntimes = ['python3-11', 'python3-12', 'python3-13', 'python3-14'];
const nodeRuntimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];

const scenarios = [
    { name: 'sqs', producerPrefix: 'tracing-sqs-producer', consumerPrefix: 'tracing-sqs-consumer', runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'sns', producerPrefix: 'tracing-sns-producer', consumerPrefix: 'tracing-sns-consumer', runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'sns-sqs', producerPrefix: 'tracing-sns-sqs-producer', consumerPrefix: 'tracing-sns-sqs-consumer', runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'kinesis', producerPrefix: 'tracing-kinesis-producer', consumerPrefix: 'tracing-kinesis-consumer', runtimes: [...pythonRuntimes] },
] as const;

const verifyTracingScenario = async (
    producerFunctionName: string,
    consumerFunctionName: string,
) => {
    // Step 1: Invoke the producer lambda
    const producerInvocationId = await invokeFunction(producerFunctionName, true, false);
    console.log(`Producer invocation ID: ${producerInvocationId}`);

    // Step 2: Fetch and verify the producer span
    let producerTraceId: string | undefined;
    let producerSpanId: string | undefined;

    const expectedScopeName = producerFunctionName.includes('python') ?
        'opentelemetry.instrumentation.aws_lambda' :
        '@opentelemetry/instrumentation-aws-lambda';

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

            // Find the lambda instrumentation scope
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

    // Step 3: Fetch and verify the consumer span has span links
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

            // Find the consumer span with span links
            let consumerSpanWithLinks: any = null;
            for (const resourceSpan of spanPayload.resourceSpans) {
                for (const scopeSpan of resourceSpan.scopeSpans) {
                    if (scopeSpan.scope.name === expectedScopeName) {
                        for (const span of scopeSpan.spans) {
                            if (span.links && span.links.length > 0) {
                                consumerSpanWithLinks = span;
                                break;
                            }
                        }
                    }
                }
            }

            expect(consumerSpanWithLinks).toBeDefined();
            console.log(`Found consumer span with ${consumerSpanWithLinks.links.length} link(s)`);

            // Verify the span link points to the producer trace
            const link = consumerSpanWithLinks.links[0];
            expect(link.traceId).toEqual(producerTraceId);
            console.log(`Span link traceId matches producer: ${link.traceId}`);
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
                `verifies ${scenario.name} trace linking for ${runtime}`,
                async () => {
                    console.log(`Starting test for ${scenario.name} scenario with ${runtime}`, new Date().toISOString());
                    await verifyTracingScenario(producerFunctionName, consumerFunctionName);
                },
                180_000
            );
        }
    }
});

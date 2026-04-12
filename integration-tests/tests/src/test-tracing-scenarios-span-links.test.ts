import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from './config';
import { findHandlerSpan, getAttributesMap, getRequestPayload, invokeFunction, RESOURCE_PREFIX } from './utils';

const pythonRuntimes = ['python3-11', 'python3-12', 'python3-13', 'python3-14'];
const nodeRuntimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
const javaRuntimes = ['java17', 'java21', 'java25'];

const scenarios = [
    { name: 'sqs', producerPrefix: `${RESOURCE_PREFIX}tracing-sqs-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-sqs-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes, ...javaRuntimes] },
    { name: 'sns', producerPrefix: `${RESOURCE_PREFIX}tracing-sns-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-sns-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes, ...javaRuntimes] },
    { name: 'sns-sqs', producerPrefix: `${RESOURCE_PREFIX}tracing-sns-sqs-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-sns-sqs-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes, ...javaRuntimes] },
    { name: 'kinesis', producerPrefix: `${RESOURCE_PREFIX}tracing-kinesis-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-kinesis-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
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
        'opentelemetry.instrumentation.aws_lambda' : producerFunctionName.includes('nodejs') ?
        '@opentelemetry/instrumentation-aws-lambda' : 'io.opentelemetry.aws-lambda-events-2.2';

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
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);

            const { span: producerSpan } = findHandlerSpan(spanPayload, expectedScopeName);

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
                    dataset: DASH0_LAMBDA_TESTS_DATASET
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
                            if (span.links && span.links.length > 0 && span.name !== "aws:sqs process") {
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

            // Verify span attributes extracted from the event payload
            const consumerAttrs = getAttributesMap(consumerSpanWithLinks.attributes);
            expect(consumerAttrs['faas.trigger']).toBeDefined();
            expect(consumerAttrs['dash0.faas.trigger_arn']).toBeDefined();
            expect(consumerAttrs['dash0.faas.record_count']).toBeDefined();
            console.log(`Consumer span attributes: faas.trigger=${JSON.stringify(consumerAttrs['faas.trigger'])}, dash0.faas.trigger_arn=${JSON.stringify(consumerAttrs['dash0.faas.trigger_arn'])}, dash0.faas.record_count=${JSON.stringify(consumerAttrs['dash0.faas.record_count'])}`);

            // Verify trigger chain attributes
            const expectedTriggerType: Record<string, string> = {
                'sqs': 'aws:sqs', 'sns': 'aws:sns', 'sns-sqs': 'aws:sns', 'kinesis': 'aws:kinesis',
            };
            const scenarioName = consumerFunctionName.includes('sns-sqs') ? 'sns-sqs' :
                consumerFunctionName.includes('sns') ? 'sns' :
                consumerFunctionName.includes('sqs') ? 'sqs' :
                consumerFunctionName.includes('kinesis') ? 'kinesis' : 'unknown';
            const expectedType = expectedTriggerType[scenarioName];

            expect(consumerAttrs['dash0.trigger.chain.depth']).toBeDefined();
            expect(consumerAttrs['dash0.trigger.chain.0.type']?.stringValue).toEqual(expectedType);
            expect(consumerAttrs['dash0.trigger.chain.0.name']).toBeDefined();

            if (scenarioName === 'sns-sqs') {
                expect(consumerAttrs['dash0.trigger.chain.depth']?.intValue).toEqual('2');
                expect(consumerAttrs['dash0.trigger.chain.1.type']?.stringValue).toEqual('aws:sqs');
                expect(consumerAttrs['dash0.trigger.chain.1.arn']).toBeDefined();
            } else {
                expect(consumerAttrs['dash0.trigger.chain.depth']?.intValue).toEqual('1');
            }
            console.log(`Trigger chain (${scenarioName}): depth=${consumerAttrs['dash0.trigger.chain.depth']?.intValue}, hop0=${consumerAttrs['dash0.trigger.chain.0.type']?.stringValue}`);

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

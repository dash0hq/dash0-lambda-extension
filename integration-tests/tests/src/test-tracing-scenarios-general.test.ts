import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { PYTHON_RUNTIMES, NODE_RUNTIMES } from '../../runtimes';
import {DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from './config';
import { getAttributesMap, RESOURCE_PREFIX } from './utils';
import { invokeProducerAndGetLeafSpan } from './utils-tracing-scenarios';

const pythonRuntimes = PYTHON_RUNTIMES.filter(r => r !== 'python3-10');
const nodeRuntimes = NODE_RUNTIMES;

const scenarios = [
    { name: 'eventbridge', producerPrefix: `${RESOURCE_PREFIX}tracing-eventbridge-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-eventbridge-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'apigateway', producerPrefix: `${RESOURCE_PREFIX}tracing-apigateway-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-apigateway-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'httpapi', producerPrefix: `${RESOURCE_PREFIX}tracing-httpapi-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-httpapi-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 's3', producerPrefix: `${RESOURCE_PREFIX}tracing-s3-producer`, consumerPrefix: `${RESOURCE_PREFIX}tracing-s3-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
    { name: 'lambda', producerPrefix: `${RESOURCE_PREFIX}tracing-lambda-invoker`, consumerPrefix: `${RESOURCE_PREFIX}tracing-lambda-consumer`, runtimes: [...pythonRuntimes, ...nodeRuntimes] },
] as const;

const fetchAndVerifyConsumerSpans = async (
    consumerFunctionName: string,
    expectedScopeName: string,
    producerTraceId: string,
    leafSpanId: string,
    scenarioName: string,
) => {
    console.log(`[trace-lookup] scenario=${scenarioName} consumer=${consumerFunctionName} producerTraceId=${producerTraceId} leafSpanId=${leafSpanId}`);
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

            // traceparent format: 00-<32 hex trace id>-<16 hex parent span id>-<flags>
            const apiCallTraceparent = spanResponse.headers.get('traceparent');
            const apiCallTraceId = apiCallTraceparent?.split('-')[1];
            console.log(`[trace-lookup] scenario=${scenarioName} consumer=${consumerFunctionName} attempt=${attempt} apiCallTraceparent=${apiCallTraceparent} apiCallTraceId=${apiCallTraceId}`);

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

            const consumerAttrs = getAttributesMap(consumerHandlerSpan!.attributes);

            if (scenarioName === 'eventbridge') {
                expect(consumerAttrs['faas.trigger']).toBeDefined();
                expect(consumerAttrs['dash0.faas.event_bridge_source']).toBeDefined();
                expect(consumerAttrs['dash0.faas.event_bridge_detail_type']).toBeDefined();
                console.log(`EventBridge attributes: faas.trigger=${JSON.stringify(consumerAttrs['faas.trigger'])}, source=${JSON.stringify(consumerAttrs['dash0.faas.event_bridge_source'])}, detail_type=${JSON.stringify(consumerAttrs['dash0.faas.event_bridge_detail_type'])}`);
            }

            // Verify trigger chain attributes for scenarios that support them
            const expectedTriggerType: Record<string, string> = {
                'eventbridge': 'aws:event_bridge', 's3': 'aws:s3',
                'apigateway': 'aws:api_gateway', 'httpapi': 'aws:api_gateway',
            };
            const expectedType = expectedTriggerType[scenarioName];
            if (expectedType) {
                expect(consumerAttrs['dash0.trigger.chain.depth']?.intValue).toEqual('1');
                expect(consumerAttrs['dash0.trigger.chain.0.type']?.stringValue).toEqual(expectedType);
                expect(consumerAttrs['dash0.trigger.chain.0.name']).toBeDefined();
                console.log(`Trigger chain (${scenarioName}): depth=1, type=${expectedType}, name=${consumerAttrs['dash0.trigger.chain.0.name']?.stringValue}`);
            }

            // Verify HTTP semconv attributes extracted from API Gateway v1/v2
            // proxy integration events, independent of the runtime SDK.
            if (scenarioName === 'apigateway' || scenarioName === 'httpapi') {
                expect(consumerAttrs['http.request.method']?.stringValue).toEqual('POST');
                expect(consumerAttrs['url.path']?.stringValue).toEqual('/');
                expect(consumerAttrs['url.scheme']?.stringValue).toEqual('https');
                expect(consumerAttrs['server.address']?.stringValue).toBeDefined();
                expect(consumerAttrs['client.address']?.stringValue).toBeDefined();
                expect(consumerAttrs['http.response.status_code']?.intValue).toEqual('200');
                // The HTTP API route is a fixed "POST /", so http.route is unambiguous.
                // The REST API is deployed with proxy:true, whose exact resource
                // matching for the root path can vary, so only require it's present.
                if (scenarioName === 'httpapi') {
                    expect(consumerAttrs['http.route']?.stringValue).toEqual('/');
                } else {
                    expect(consumerAttrs['http.route']).toBeDefined();
                }
                console.log(`HTTP attributes (${scenarioName}): method=${consumerAttrs['http.request.method']?.stringValue}, route=${consumerAttrs['http.route']?.stringValue}, status_code=${consumerAttrs['http.response.status_code']?.intValue}`);
            }

            return;
        } catch (error) {
            console.error(`Error fetching consumer spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                console.error(`[trace-lookup] GAVE UP scenario=${scenarioName} consumer=${consumerFunctionName} producerTraceId=${producerTraceId} leafSpanId=${leafSpanId}`);
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
    const { producerTraceId, leafSpanId, expectedConsumerScopeName } =
        await invokeProducerAndGetLeafSpan(producerFunctionName, consumerFunctionName);

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
                360_000
            );
        }
    }
});

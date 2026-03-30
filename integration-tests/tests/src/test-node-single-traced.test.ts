import { describe, expect, it } from 'vitest';
import { setTimeout as delay } from 'timers/promises';
import fetch from 'node-fetch';
import {
    checkResourceAttributes,
    getAttributesMap,
    invokeFunction,
    RESOURCE_PREFIX,
} from './utils';
import { DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from './config';

const getRequestPayload = (filter: Array<{ operator: string; key: string; value: string }>) => {
    const now = Date.now();
    return {
        filter,
        timeRange: {
            from: new Date(now - 5 * 60_000).toISOString(),
            to: new Date(now + 5 * 60_000).toISOString(),
        },
        sampling: { mode: 'adaptive' },
        dataset: DASH0_LAMBDA_TESTS_DATASET,
    };
};

const fetchSpans = async (filter: Array<{ operator: string; key: string; value: string }>) => {
    const response = await fetch(DASH0_ENDPOINT + 'spans', {
        method: 'POST',
        headers: {
            accept: 'application/json',
            authorization: `Bearer ${DASH0_TOKEN}`,
            'content-type': 'application/json',
        },
        body: JSON.stringify(getRequestPayload(filter)),
    });
    return (await response.json()) as any;
};

const flattenSpans = (spanPayload: any): Array<{ span: any; resource: any }> => {
    const results: Array<{ span: any; resource: any }> = [];
    for (const rs of (spanPayload?.resourceSpans ?? [])) {
        for (const ss of (rs.scopeSpans ?? [])) {
            for (const span of (ss.spans ?? [])) {
                results.push({ span, resource: rs.resource });
            }
        }
    }
    return results;
};

const verifySingleTracedInvocation = async (functionName: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    // Step 1: Find handler span by faas.invocation_id
    let handlerSpan: any = null;
    let handlerResource: any = null;
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch handler span for invocation ID ${invocationId}`);
        try {
            const spanPayload = await fetchSpans([
                { operator: 'is', key: 'faas.invocation_id', value: invocationId },
            ]);

            const allSpans = flattenSpans(spanPayload);
            expect(allSpans.length).toBeGreaterThanOrEqual(1);

            // The handler span has name 'handler'
            const found = allSpans.find(({ span }) => span.name === 'handler');
            expect(found, 'Handler span with name "handler" not found').toBeDefined();
            handlerSpan = found!.span;
            handlerResource = found!.resource;
            break;
        } catch (error) {
            console.error(`Error fetching handler span on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) throw error;
        }
    }

    const traceId = handlerSpan.traceId;
    const handlerParentSpanId = handlerSpan.parentSpanId;

    // Service and execution environment spans are sent on the next invocation, so trigger one
    await invokeFunction(functionName, true, false);

    // Step 2: Query by service.name to find the service span and execution environment span
    let serviceSpan: any = null;
    let serviceResource: any = null;
    let executionSpan: any = null;
    let executionResource: any = null;
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch spans by service.name for ${functionName}`);
        try {
            const spanPayload = await fetchSpans([
                { operator: 'is', key: 'service.name', value: functionName },
            ]);

            const allSpans = flattenSpans(spanPayload);
            console.log(`Found ${allSpans.length} total spans for service.name=${functionName}`);
            for (const { span } of allSpans) {
                console.log(`  span: name=${span.name}, spanId=${span.spanId}, parentSpanId=${span.parentSpanId}, traceId=${span.traceId}`);
            }
            console.log(`Looking for traceId=${traceId}, handlerParentSpanId=${handlerParentSpanId}`);
            // Filter to spans belonging to the same trace
            const traceSpans = allSpans.filter(({ span }) => span.traceId === traceId);
            console.log(`Found ${traceSpans.length} spans in same trace`);

            // Execution environment span: its spanId is the handler's parentSpanId
            const execFound = traceSpans.find(({ span }) => span.spanId === handlerParentSpanId);
            expect(execFound, 'Execution environment span not found').toBeDefined();
            executionSpan = execFound!.span;
            executionResource = execFound!.resource;

            // Service span: its spanId is the execution span's parentSpanId
            const svcFound = traceSpans.find(({ span }) => span.spanId === executionSpan.parentSpanId);
            expect(svcFound, 'Service span not found').toBeDefined();
            serviceSpan = svcFound!.span;
            serviceResource = svcFound!.resource;
            break;
        } catch (error) {
            console.error(`Error fetching service.name spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) throw error;
        }
    }

    // Verify trace structure: service -> execution -> handler
    expect(serviceSpan.traceId).toEqual(traceId);
    expect(executionSpan.traceId).toEqual(traceId);
    expect(handlerSpan.traceId).toEqual(traceId);

    expect(executionSpan.parentSpanId).toEqual(serviceSpan.spanId);
    expect(handlerSpan.parentSpanId).toEqual(executionSpan.spanId);

    // Verify resource attributes
    checkResourceAttributes(handlerResource.attributes, functionName);
    checkResourceAttributes(executionResource.attributes, functionName);
    checkResourceAttributes(serviceResource.attributes, functionName);

    // Verify service.name on all resources
    const handlerResAttrs = getAttributesMap(handlerResource.attributes);
    expect(handlerResAttrs['service.name'].stringValue).toEqual(functionName);
    const execResAttrs = getAttributesMap(executionResource.attributes);
    expect(execResAttrs['service.name'].stringValue).toEqual(functionName);
    const svcResAttrs = getAttributesMap(serviceResource.attributes);
    expect(svcResAttrs['service.name'].stringValue).toEqual(functionName);
};

describe.concurrent('Single-traced Lambda invocation', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}single-traced-${runtime}`;
        it(
            `invokes ${functionName} successfully`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifySingleTracedInvocation(functionName);
            },
            120_000,
        );
    }
});

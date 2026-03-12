import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import {
    checkException,
    checkLogs,
    checkResourceAttributes,
    checkSupplementarySpans, LogToCheck,
    getAttributesMap,
    getRequestPayload,
    invokeFunction, runAllTests
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    let traceId: string | undefined = undefined;
    let parentSpanId: string | undefined = undefined;
    let rootSpanId: string | undefined = undefined;
    let span = undefined
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
            // Find the Lambda instrumentation scope (supplementary spans may also be present)
            let lambdaScopeSpan = null;
            let lambdaResource = null;
            for (const rs of (spanPayload?.resourceSpans ?? [])) {
                for (const ss of (rs.scopeSpans ?? [])) {
                    if (ss.scope?.name === 'opentelemetry.instrumentation.aws_lambda') {
                        lambdaScopeSpan = ss;
                        lambdaResource = rs.resource;
                    }
                }
            }
            expect(lambdaScopeSpan).toBeDefined();
            expect(lambdaScopeSpan!.spans.length).toEqual(1);
            const resourceAttributes = getAttributesMap(lambdaResource!.attributes);
            expect(resourceAttributes['service.name'].stringValue).toEqual(functionName);
            checkResourceAttributes(lambdaResource!.attributes, functionName);
            // check span attributes
            span = lambdaScopeSpan!.spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            traceId = span.traceId;
            parentSpanId = span.spanId;
            rootSpanId = span.parentSpanId;
            checkException(span, 'timeout');
            break;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
    ]
    if (!invocationEnd) {
        logsToBeChecked.push({ message: "Status: timeout" });
    }
    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: parentSpanId!,
        success: false,
        logsToBeChecked
    });

    // Supplementary spans are sent on the next invocation; trigger one if needed
    if (invocationEnd) {
        await invokeFunction(functionName, true, true);
    }
    await checkSupplementarySpans({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        rootSpanId: rootSpanId!,
        runtimeError: true,
    });
}

describe.concurrent('Lambdainvocation java timeout', () => {
    const runtimes = ['java17', 'java21', 'java25'];
    runAllTests('timeout', runtimes, verifySuccessInvocation);
});

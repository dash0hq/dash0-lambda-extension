import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS } from "./config";
import {
    checkHttpSpan,
    checkLogs,
    checkResourceAttributes,
    checkSupplementarySpans,
    getAttributesMap, LogToCheck,
    getRequestPayload,
    invokeFunction, runAllTests
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, false);

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
            // check span attributes
            span = lambdaScopeSpan!.spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);

            checkResourceAttributes(lambdaResource!.attributes, functionName);
            const resourceAttributes = getAttributesMap(lambdaResource!.attributes);
            expect(resourceAttributes['process.environment_variable.DASH0_TOKEN'].stringValue).toEqual('****');

            traceId = span.traceId;
            parentSpanId = span.spanId;
            rootSpanId = span.parentSpanId;
            break;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    let httpSpanId: string | undefined = undefined;
    if (traced) {
        httpSpanId = await checkHttpSpan({
            invocationId: invocationId!,
            functionName,
            traceId: traceId!,
            parentSpanId: parentSpanId!,
        });
    }
    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: "response.status_code:" },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200,  body: '"Hello from Lambda!"' } }), isJson: true },
    ]
    if (traced) {
        logsToBeChecked.push(
            { message: JSON.stringify({ name: "dash0_payload", type: "http_request_body", message: { title: "foo", userId: 1 } }), isJson: true, spanId: httpSpanId },
            { message: JSON.stringify({ name: "dash0_payload", type: "http_response_body" }), isJson: true, spanId: httpSpanId },
        );
    }
    if (!invocationEnd) {
        logsToBeChecked.push({ message: 'REPORT RequestId: ' });
    }
    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: rootSpanId!,
        success: true,
        logsToBeChecked
    });

    // Supplementary spans are sent on the next invocation; trigger one if needed
    if (invocationEnd) {
        await invokeFunction(functionName, true, false);
    }
    await checkSupplementarySpans({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        rootSpanId: rootSpanId!,
    });
}

describe.concurrent('Lambdainvocation', () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];
    runAllTests('success', runtimes, verifySuccessInvocation);
});

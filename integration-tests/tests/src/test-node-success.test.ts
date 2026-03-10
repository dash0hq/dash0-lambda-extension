import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {
    checkHttpSpan,
    checkLogs,
    checkResourceAttributes,
    checkSpanAttributesFromReport,
    checkSupplementarySpans,
    getAttributesMap,
    getRequestPayload,
    invokeFunction, LogToCheck, runAllTests
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationPayload = JSON.stringify({ parameter1: 'right', masked_field: 'this should not be seen!' });
    const invocationId = await invokeFunction(functionName, invocationEnd, false, invocationPayload);

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
            const expectedScopeName = traced ? "@opentelemetry/instrumentation-aws-lambda" : "opentelemetry.instrumentation.aws_lambda";
            let lambdaScopeSpan = null;
            let lambdaResource = null;
            for (const rs of (spanPayload?.resourceSpans ?? [])) {
                for (const ss of (rs.scopeSpans ?? [])) {
                    if (ss.scope?.name === expectedScopeName) {
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

            const resourceAttributes = getAttributesMap(lambdaResource!.attributes);
            expect(JSON.parse(resourceAttributes['process.environ'].stringValue)['MASKED_FIELD']).toEqual('****');
            checkResourceAttributes(lambdaResource!.attributes, functionName);

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
        { message: "Handler invoked with event:" },
        { message: "let's parse this as a warning", severity: "warn" },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right", masked_field: "****" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200 } }), isJson: true },
    ]
    if (traced) {
        logsToBeChecked.push(
            { message: JSON.stringify({ name: "dash0_payload", type: "http_request_body", message: { title: "foo", body: "bar", userId: 1 } }), isJson: true, spanId: httpSpanId },
            { message: JSON.stringify({ name: "dash0_payload", type: "http_response_body" }), isJson: true, spanId: httpSpanId },
        );
    }
    if (!invocationEnd) {
        logsToBeChecked.push({ message: 'REPORT RequestId: ' });
    }
    const reportLog = await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: traceId!,
        parentSpanId: parentSpanId!,
        success: true,
        logsToBeChecked
    });
    if (!invocationEnd) {
        checkSpanAttributesFromReport(reportLog, span);
    }

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

describe.concurrent('Lambda invocation', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    runAllTests('success', runtimes, verifySuccessInvocation);
});

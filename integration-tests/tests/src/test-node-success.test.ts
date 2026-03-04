import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {
    checkHttpSpan,
    checkLogs,
    checkResourceAttributes,
    checkSpanAttributesFromReport, compareJsonStrings,
    getAttributesMap,
    getRequestPayload,
    invokeFunction, LogToCheck, runAllTests
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationPayload = JSON.stringify({ parameter1: 'right', masked_field: 'this should not be seen!' });
    const invocationId = await invokeFunction(functionName, invocationEnd, false, invocationPayload);

    let traceId: string | undefined = undefined;
    let parentSpanId: string | undefined = undefined;
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
            expect(spanPayload?.resourceSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toEqual(1);
            const expectedScopeName = traced ? "@opentelemetry/instrumentation-aws-lambda" : "opentelemetry.instrumentation.aws_lambda";
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual(expectedScopeName);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);
            // check span attributes
            span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            compareJsonStrings(spanAttributes['dash0.faas.event'].stringValue, '{"parameter1":"right","masked_field":"****"}');
            compareJsonStrings(spanAttributes['dash0.faas.return_value'].stringValue, '{"statusCode":200,"body":"{\\"message\\":\\"Success\\"}"}');

            const resourceAttributes = getAttributesMap(spanPayload.resourceSpans[0].resource.attributes);
            expect(JSON.parse(resourceAttributes['process.environ'].stringValue)['MASKED_FIELD']).toEqual('****');
            checkResourceAttributes(spanPayload.resourceSpans[0].resource.attributes, functionName);

            traceId = span.traceId;
            parentSpanId = span.spanId;
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
        { message: "Handler invoked with event:" },
        { message: "let's parse this as a warning", severity: "warn" },
        { message: 'END RequestId: ' },
    ]
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
}

describe.concurrent('Lambda invocation', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    runAllTests('success', runtimes, verifySuccessInvocation);
});

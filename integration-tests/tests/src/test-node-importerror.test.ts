import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { NODE_RUNTIMES } from '../../runtimes';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {checkException, checkLogs, getAttributesMap, getRequestPayload, invokeFunction, LogToCheck, runAllTests} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

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
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual("opentelemetry.instrumentation.aws_lambda");
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);

            // check resource attributes
            const resourceAttrMap = getAttributesMap(spanPayload.resourceSpans[0].resource.attributes);
            expect(resourceAttrMap['cloud.resource_id'].stringValue).toContain(functionName);

            // check span attributes
            const span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            checkException(span, 'Runtime.Unknown');
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
    ];
    if (!invocationEnd) {
        logsToBeChecked.push({ message: "Error Type: Runtime.Unknown" });
    }
    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: null,
        parentSpanId: null,
        success: false,
        logsToBeChecked,
    });
}

describe.concurrent('Lambda invocations with importerror', {retry: 1}, () => {
    const runtimes = NODE_RUNTIMES;
    runAllTests('importerror', runtimes, verifySuccessInvocation);
});

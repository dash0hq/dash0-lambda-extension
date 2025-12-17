import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {
    checkException, checkHttpSpan, checkLogs,
    checkSpanAttributesFromReport, getAttributesMap, getRequestPayload, invokeFunction
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

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
            expect(spanAttributes['faas.event'].stringValue).toEqual('{"parameter1":"right"}');
            expect(spanAttributes['faas.return_value'].stringValue).contains('ReferenceError');

            // check exception event
            const events = span.events;
            expect(events.length).toEqual(1);
            const exceptionEvent = events[0];
            expect(exceptionEvent.name).toEqual('exception');
            const eventAttributes = exceptionEvent.attributes;
            const eventAttrMap: Record<string, any> = {};
            for (const attr of eventAttributes) {
                eventAttrMap[attr.key] = attr.value;
            }
            expect(eventAttrMap['exception.type'].stringValue).toEqual('ReferenceError');
            expect(span.status.code).toEqual(2); // 2 = ERROR
            expect(span.status.message).toEqual(traced ? 'nothing is not defined' : 'ReferenceError');

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
    const logsToBeChecked = [
        'START RequestId: ',
        "Handler invoked with event:",
        "ReferenceError",
        'END RequestId: ',
    ]
    if (!invocationEnd) {
        logsToBeChecked.push('REPORT RequestId: ');
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
    const architectures = ['x86_64', 'arm64'] as const;
    const invocationEndValues = [true, false] as const;
    const tracedValues = [true, false] as const;

    for (const runtime of runtimes) {
        for (const architecture of architectures) {
            for (const invocationEnd of invocationEndValues) {
                for (const traced of tracedValues) {
                    const invocationEndLabel = invocationEnd ? 'true' : 'false';
                    const functionName = `${runtime}-exception-${traced}-invocation-end-${invocationEndLabel}-${architecture}`;
                    it(
                        `invokes ${functionName} successfully`,
                        async () => {
                            console.log(`Starting test for ${functionName}`, new Date().toISOString());
                            await verifySuccessInvocation(functionName, invocationEnd, traced);
                        },
                        120_000
                    );
                }
            }
        }
    }
});

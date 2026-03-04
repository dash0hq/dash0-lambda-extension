import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {
    checkException, checkHttpSpan, checkLogs,
    checkSpanAttributesFromReport, getAttributesMap, getRequestPayload, invokeFunction, LogToCheck, runAllTests
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true, JSON.stringify({ parameter1: 'throw' }));

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
            const expectedScopeName = traced ? "io.opentelemetry.aws-lambda-events-2.2" : "opentelemetry.instrumentation.aws_lambda";
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual(expectedScopeName);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);
            const resourceAttributes = getAttributesMap(spanPayload?.resourceSpans[0].resource.attributes);
            expect(resourceAttributes['service.name'].stringValue).toEqual(functionName);
            // check span attributes
            span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);
            expect(spanAttributes['dash0.faas.event'].stringValue).toEqual('{"parameter1":"throw"}');
            // expect(spanAttributes['dash0.faas.return_value'].stringValue).contains('ReferenceError');


            // check exception event
            const events = span.events;
            console.log(events);
            console.log(spanAttributes);
            expect(events.length).toEqual(1);
            const exceptionEvent = events[0];
            expect(exceptionEvent.name).toEqual('exception');
            const eventAttributes = exceptionEvent.attributes;
            const eventAttrMap: Record<string, any> = {};
            for (const attr of eventAttributes) {
                eventAttrMap[attr.key] = attr.value;
            }
            expect(eventAttrMap['exception.type'].stringValue).toEqual('java.lang.RuntimeException');
            expect(eventAttrMap['exception.message'].stringValue).toEqual("Intentional exception triggered by input 'throw'");
            expect(span.status.code).toEqual(2); // 2 = ERROR
            expect(span.status.message).toEqual(traced ? "" : 'java.lang.RuntimeException');

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
        { message: "Input received:" },
        { message: "java.lang.RuntimeException" },
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
    const runtimes = ['java17', 'java21', 'java25'];
    runAllTests('exception', runtimes, verifySuccessInvocation);
});

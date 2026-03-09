import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {
    checkHttpSpan,
    checkLogs,
    checkSpanAttributesFromReport,
    getAttributesMap,
    getRequestPayload, LogToCheck,
    invokeFunction, runAllTests
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
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].scope.name).toEqual("opentelemetry.instrumentation.aws_lambda");
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toEqual(1);
            // check span attributes
            span = spanPayload.resourceSpans[0].scopeSpans[0].spans[0];
            const spanAttributes = getAttributesMap(span.attributes);
            expect(spanAttributes['faas.invocation_id'].stringValue).toEqual(invocationId);

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
            expect(eventAttrMap['exception.type'].stringValue).toEqual('KeyError');
            expect(span.status.code).toEqual(2); // 2 = ERROR
            expect(span.status.message).toEqual(traced ? '' : 'KeyError');

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
        { message: "[ERROR] KeyError:" },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value" }), isJson: true },
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
}

describe.concurrent('Lambda invocation', () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];
    runAllTests('exception', runtimes, verifySuccessInvocation);
});

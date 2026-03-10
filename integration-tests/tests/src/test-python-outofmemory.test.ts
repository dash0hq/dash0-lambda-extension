import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {checkLogs, checkSupplementarySpans, getAttributesMap, getRequestPayload, invokeFunction, LogToCheck, runAllTests} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    let rootSpanId: string | undefined = undefined;
    let traceId: string | undefined = undefined;
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
            for (const rs of (spanPayload?.resourceSpans ?? [])) {
                for (const ss of (rs.scopeSpans ?? [])) {
                    if (ss.scope?.name === 'opentelemetry.instrumentation.aws_lambda') {
                        lambdaScopeSpan = ss;
                    }
                }
            }
            expect(lambdaScopeSpan).toBeDefined();
            expect(lambdaScopeSpan!.spans.length).toEqual(1);
            // check span attributes
            const span = lambdaScopeSpan!.spans[0];
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
            expect(eventAttrMap['exception.type'].stringValue).toEqual('Runtime.OutOfMemory');
            expect(span.status.code).toEqual(2); // 2 = ERROR
            expect(span.status.message).toEqual('Runtime.OutOfMemory');
            traceId = span.traceId;
            rootSpanId = span.parentSpanId;
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
        { message: "response.status_code:" },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
    ];
    if (!invocationEnd) {
        logsToBeChecked.push({ message: "Runtime.OutOfMemory" });
    }
    await checkLogs({
        invocationId: invocationId!,
        functionName,
        traceId: null,
        parentSpanId: null,
        success: false,
        logsToBeChecked,
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

describe.concurrent('Lambda invocations with outofmemory', {retry: 1}, () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];
    runAllTests('outofmemory', runtimes, verifySuccessInvocation);
});

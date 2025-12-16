import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {checkException, checkLogs, getAttributesMap, getRequestPayload, invokeFunction} from "./utils";


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
            checkException(span, 'Runtime.ImportModuleError');
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
        'END RequestId: ',
    ];
    if (!invocationEnd) {
        logsToBeChecked.push("Error Type: Runtime.ImportModuleError", "REPORT RequestId: ");
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
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];
    const architectures = ['x86_64', 'arm64'] as const;
    const tracedValues = [true, false] as const;
    const invocationEndValues = [true, false] as const;

    for (const runtime of runtimes) {
        for (const architecture of architectures) {
            for (const traced of tracedValues) {
                for (const invocationEnd of invocationEndValues) {
                    const functionName = `${runtime}-importerror-${traced}-invocation-end-${invocationEnd}-${architecture}`;
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

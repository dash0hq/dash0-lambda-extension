import {setTimeout as delay} from "timers/promises";
import fetch from "node-fetch";
import {expect} from "vitest";
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {InvokeCommand, LambdaClient} from "@aws-sdk/client-lambda";

const lambdaClient = new LambdaClient({
    region: process.env.AWS_REGION ?? 'us-west-2',
});

export const getAttributesMap = (attributes: Array<{ key: string, value: any }>) => {
    const attrMap: Record<string, any> = {};
    for (const attr of attributes) {
        attrMap[attr.key] = attr.value;
    }
    return attrMap;
}

export const getRequestPayload = (invocationId: string) => {
    const now = Date.now();
    return {
        filter: [
            {
                operator: 'is',
                key: 'faas.invocation_id',
                value: invocationId,
            },
        ],
        timeRange: {
            from: new Date(now - 5 * 60_000).toISOString(),
            to: new Date(now + 5 * 60_000).toISOString(),
        },
        sampling: { mode: 'adaptive' },
    };
}

export const invokeFunction = async (functionName: string, invocationEnd: boolean, expectError: boolean) : Promise<string> => {
    const payload = JSON.stringify({ parameter1: 'right' });

    const response = await lambdaClient.send(
        new InvokeCommand({
            FunctionName: functionName,
            InvocationType: 'RequestResponse',
            Payload: Buffer.from(payload),
        })
    );

    if (!invocationEnd) {
        await delay(2000);
        await lambdaClient.send(
            new InvokeCommand({
                FunctionName: functionName,
                InvocationType: 'RequestResponse',
                Payload: Buffer.from(payload),
            })
        );
    }

    expect(response.StatusCode).toBeLessThan(300);
    if (expectError) {
        expect(response.FunctionError).toBeDefined();
    } else {
        expect(response.FunctionError).toBeUndefined();
    }
    const invocationId = response.$metadata.requestId;
    expect(invocationId).toBeDefined();
    return invocationId!;
}


export const checkHttpSpan = async ({
     invocationId,
     functionName,
     traceId,
     parentSpanId,
 }: {
    invocationId: string,
    functionName: string,
    traceId: string,
    parentSpanId: string,
}) => {
    const now = Date.now();
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch http spans for invocation ID ${invocationId}`);
        try {
            const spanResponse = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify({
                    filter: [
                        {
                            operator: 'contains',
                            key: 'http.response.body',
                            value: functionName,
                        },
                    ],
                    timeRange: {
                        from: new Date(now - 5 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: {mode: 'adaptive'},
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toBeGreaterThanOrEqual(1);
            // find span with matching traceId and parentSpanId
            const httpSpans = spanPayload.resourceSpans[0].scopeSpans[0].spans;
            const matchingSpan = httpSpans.find((span: any) => span.traceId === traceId && span.parentSpanId === parentSpanId);
            expect(matchingSpan).toBeDefined();
            break;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
}

export const checkLogs = async ({
    invocationId,
    functionName,
    traceId,
    parentSpanId,
    success,
    logsToBeChecked,
}: {
    invocationId: string,
    functionName: string,
    traceId: string | null,
    parentSpanId: string | null,
    success: boolean,
    logsToBeChecked: string[],
}) => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch logs for invocation ID ${invocationId}`);
        try {
            const spanResponse = await fetch(DASH0_ENDPOINT + 'logs', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify(getRequestPayload(invocationId)),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceLogs.length).toEqual(1);
            const resourceAttributes = getAttributesMap(spanPayload?.resourceLogs[0].resource.attributes);
            expect(resourceAttributes['cloud.resource.id'].stringValue).toContain(functionName);

            expect(spanPayload?.resourceLogs[0].scopeLogs[0].logRecords.length).toBeGreaterThanOrEqual(2);
            const logsToBeCheckedCount: {[key: string]: boolean} = {};
            for (const logRecord of spanPayload?.resourceLogs[0].scopeLogs[0].logRecords) {
                if (traceId && parentSpanId && (success || !logRecord.body.stringValue.startsWith("REPORT RequestId: "))) {
                    // on error report doesn't have traceId and spanId associated because it arrives after shutdown, data erased.
                    expect(logRecord.traceId).toEqual(traceId);
                    expect(logRecord.spanId).toEqual(parentSpanId);
                }
                for (const logMessage of logsToBeChecked) {
                    if (logRecord.body.stringValue.includes(logMessage)) {
                        logsToBeCheckedCount[logMessage] = true;
                    }
                }
            }
            for (const logMessage of logsToBeChecked) {
                expect(logsToBeCheckedCount[logMessage]).toBeTruthy();
            }
            break;
        } catch (error) {
            console.error(`Error fetching logs on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
}
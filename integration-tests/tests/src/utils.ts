import {setTimeout as delay} from "timers/promises";
import fetch from "node-fetch";
import {expect, it} from "vitest";
import {DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {InvokeCommand, LambdaClient} from "@aws-sdk/client-lambda";

export const RESOURCE_PREFIX = process.env.RESOURCE_PREFIX ?? '';

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

export const invokeFunction = async (
    functionName: string, invocationEnd: boolean, expectError: boolean, eventPayload?: string
) : Promise<string> => {
    const payload = eventPayload ? eventPayload : JSON.stringify({ parameter1: 'right' });

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
}): Promise<string> => {
    let reportLog = null;
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
                if (logRecord.body.stringValue.startsWith("REPORT RequestId: ")) {
                    reportLog = logRecord.body.stringValue;
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
    return reportLog;
}

export const checkException = (span: any, exception_type: string) => {
    const events = span.events;
    expect(events.length).toEqual(1);
    const exceptionEvent = events[0];
    expect(exceptionEvent.name).toEqual('exception');
    const eventAttributes = exceptionEvent.attributes;
    const eventAttrMap: Record<string, any> = {};
    for (const attr of eventAttributes) {
        eventAttrMap[attr.key] = attr.value;
    }
    expect(eventAttrMap['exception.type'].stringValue).toEqual(exception_type);
    expect(span.status.code).toEqual(2); // 2 = ERROR
    expect(span.status.message).toEqual(exception_type);
}

export const checkSpanAttributesFromReport = (reportLog: string, span: any) => {
    const spanAttributes = getAttributesMap(span.attributes);
    const reportRegex = /REPORT RequestId: (?<requestId>[a-f0-9\-]+)\s+Duration: (?<duration>[\d\.]+) ms\s+Billed Duration: (?<billedDuration>[\d\.]+) ms\s+Memory Size: (?<memorySize>\d+) MB\s+Max Memory Used: (?<maxMemoryUsed>\d+) MB\s+Init Duration: (?<initDuration>[\d\.]+) ms/;
    const match = reportLog.match(reportRegex);
    if (match?.groups) {
        const initDuration = parseFloat(match.groups.initDuration);
        const billedDuration = parseFloat(match.groups.billedDuration);
        const maxMemoryUsed = parseInt(match.groups.maxMemoryUsed);
        const duration = parseFloat(match.groups.duration);

        expect(initDuration).toBeCloseTo(spanAttributes['dash0.faas.init_duration'].doubleValue, 0);
        expect(billedDuration).toEqual(spanAttributes['dash0.faas.billed_duration'].doubleValue);
        expect(maxMemoryUsed).toEqual(Number(spanAttributes['dash0.faas.memory_used'].intValue));

        const spanDurationNano = Number(BigInt(span.endTimeUnixNano) - BigInt(span.startTimeUnixNano));
        const spanDurationMs = spanDurationNano / 1e6;
        expect(spanDurationMs).toBeGreaterThanOrEqual(duration - 2);
        expect(spanDurationMs).toBeLessThanOrEqual(duration + 2);
    } else {
        throw new Error("Failed to parse REPORT log: " + reportLog);
    }
}

export const compareJsonStrings = (json1: string, json2: string) => {
    const obj1 = JSON.parse(json1);
    const obj2 = JSON.parse(json2);
    expect(obj1).toEqual(obj2);
}

export const runAllTests = (scenario: string, runtimes: string[], verifySuccessInvocation: Function) => {
    const architectures = ['x86_64', 'arm64'] as const;
    const invocationEndValues = [true, false] as const;
    const tracedValues = [true, false] as const;

    for (const runtime of runtimes) {
        for (const architecture of architectures) {
            for (const invocationEnd of invocationEndValues) {
                for (const traced of tracedValues) {
                    const invocationEndLabel = invocationEnd ? 'true' : 'false';
                    const functionName = `${RESOURCE_PREFIX}${runtime}-${scenario}-${traced}-invocation-end-${invocationEndLabel}-${architecture}`;
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
}
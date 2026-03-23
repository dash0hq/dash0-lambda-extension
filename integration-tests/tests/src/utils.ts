import {setTimeout as delay} from "timers/promises";
import fetch from "node-fetch";
import {expect, it} from "vitest";
import {DASH0_ENDPOINT, DASH0_LAMBDA_TESTS_DATASET, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS} from "./config";
import {InvokeCommand, LambdaClient} from "@aws-sdk/client-lambda";

export type LogToCheck = { message: string; severity?: string; isJson?: boolean; spanId?: string; attributes?: Record<string, string> };

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

export const findHandlerSpan = (spanPayload: any, scopeName: string): { span: any; resource: any } => {
    for (const rs of spanPayload.resourceSpans) {
        for (const ss of rs.scopeSpans) {
            if (ss.scope.name === scopeName) {
                expect(ss.spans.length).toEqual(1);
                return { span: ss.spans[0], resource: rs.resource };
            }
        }
    }
    throw new Error(`Handler span not found in scope ${scopeName}`);
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
        dataset: DASH0_LAMBDA_TESTS_DATASET
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
        await delay(4000);
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
                            operator: 'is',
                            key: 'service.name',
                            value: functionName,
                        },
                        {
                            operator: 'is',
                            key: 'http.request.method',
                            value: 'POST',
                        },
                    ],
                    timeRange: {
                        from: new Date(now - 5 * 60_000).toISOString(),
                        to: new Date(now + 5 * 60_000).toISOString(),
                    },
                    sampling: {mode: 'adaptive'},
                    dataset: DASH0_LAMBDA_TESTS_DATASET
                }),
            });

            const spanPayload = await spanResponse.json() as any;
            expect(spanPayload?.resourceSpans.length).toBeGreaterThanOrEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans.length).toEqual(1);
            expect(spanPayload?.resourceSpans[0].scopeSpans[0].spans.length).toBeGreaterThanOrEqual(1);
            // find span with matching traceId and parentSpanId
            const httpSpans = spanPayload.resourceSpans[0].scopeSpans[0].spans;
            const matchingSpan = httpSpans.find((span: any) => span.traceId === traceId && span.parentSpanId === parentSpanId);
            expect(matchingSpan).toBeDefined();
            checkResourceAttributes(spanPayload.resourceSpans[0].resource.attributes, functionName);
            return matchingSpan.spanId;
        } catch (error) {
            console.error(`Error fetching spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
}

const deepPartialMatch = (actual: any, expected: any): boolean => {
    if (expected === null || expected === undefined) return actual === expected;
    if (typeof expected !== 'object') return actual === expected;
    if (typeof actual !== 'object' || actual === null) return false;
    return Object.keys(expected).every(key => deepPartialMatch(actual[key], expected[key]));
};

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
    logsToBeChecked: LogToCheck[],
}): Promise<void> => {
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
            expect(spanPayload?.resourceLogs.length).toBeGreaterThanOrEqual(1);
            checkResourceAttributes(spanPayload.resourceLogs[0].resource.attributes, functionName);

            const allLogRecords = spanPayload?.resourceLogs.flatMap((rl: any) =>
                rl.scopeLogs.flatMap((sl: any) => sl.logRecords)
            );
            expect(allLogRecords.length).toBeGreaterThanOrEqual(2);
            const logsToBeCheckedCount: {[key: string]: boolean} = {};
            for (const logRecord of allLogRecords) {
                let expectedSeverity = "info";
                let hasJsonMatch = false;
                let jsonSeverity: string | null = null;
                let matchedSpanId: string | null = null;
                let matchedCheck: LogToCheck | null = null;
                let matched = false;
                for (const logToCheck of logsToBeChecked) {
                    if (logToCheck.isJson) {
                        try {
                            const actual = JSON.parse(logRecord.body.stringValue);
                            const expected = JSON.parse(logToCheck.message);
                            matched = deepPartialMatch(actual, expected);
                        } catch {
                            matched = false;
                        }
                    } else {
                        matched = logRecord.body.stringValue.includes(logToCheck.message);
                    }
                    if (matched) {
                        logsToBeCheckedCount[logToCheck.message] = true;
                        matchedCheck = logToCheck;
                        if (logToCheck.spanId) {
                            matchedSpanId = logToCheck.spanId;
                        }
                        if (logToCheck.isJson) {
                            hasJsonMatch = true;
                            if (logToCheck.severity) {
                                jsonSeverity = logToCheck.severity;
                            }
                        } else if (logToCheck.severity) {
                            expectedSeverity = logToCheck.severity;
                        }
                        break;
                    }
                }
                if (!matched) {
                    continue;
                }

                // Verify trace/span IDs
                if (matchedSpanId) {
                    // Log matched a check with a specific spanId (e.g. HTTP body payload logs)
                    if (traceId) {
                        expect(logRecord.traceId).toEqual(traceId);
                    }
                    expect(logRecord.spanId).toEqual(matchedSpanId);
                } else if (traceId && parentSpanId && (success || !logRecord.body.stringValue.startsWith("REPORT RequestId: "))) {
                    // on error report doesn't have traceId and spanId associated because it arrives after shutdown, data erased.
                    expect(logRecord.traceId).toEqual(traceId);
                    expect(logRecord.spanId).toEqual(parentSpanId);
                }

                // If a JSON check matched this log record, use its severity (or default "info"),
                // ignoring severity from non-JSON includes matches that may have matched incidentally.
                if (hasJsonMatch) {
                    expectedSeverity = jsonSeverity ?? "info";
                }
                expect(logRecord.severityText.toLowerCase(), `Wrong severity: ${JSON.stringify(logRecord)}`).toEqual(expectedSeverity);

                // Verify custom attributes if specified
                if (matchedCheck?.attributes) {
                    const logAttrs = getAttributesMap(logRecord.attributes);
                    for (const [key, value] of Object.entries(matchedCheck.attributes)) {
                        expect(logAttrs[key]?.stringValue, `Missing or wrong attribute '${key}' on log: ${JSON.stringify(logRecord)}`).toEqual(value);
                    }
                }
            }
            for (const logToCheck of logsToBeChecked) {
                expect(logsToBeCheckedCount[logToCheck.message], `Log not found: ${logToCheck.message}`).toBeTruthy();
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

export const checkResourceAttributes = (attributes: Array<{ key: string, value: any }>, functionName: string) => {
    const resourceAttributes = getAttributesMap(attributes);
    expect(resourceAttributes['cloud.platform'].stringValue).toEqual('aws_lambda');
    expect(resourceAttributes['cloud.resource_id'].stringValue).toContain(functionName);
    expect(resourceAttributes['cloud.account.id'].stringValue).toMatch(/^\d+$/);
}

export type MainSpansResult = {
    traceId: string;
    rootSpanId: string;
    handlerSpanId: string;
    handlerSpan: any;
    resource: any;
};

export const checkMainSpans = async ({
    invocationId,
    functionName,
    handlerScopeName,
}: {
    invocationId: string;
    functionName: string;
    handlerScopeName: string;
}): Promise<MainSpansResult> => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch main spans for invocation ID ${invocationId}`);
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

            // Find handler span in the handler scope
            let handlerSpan: any = null;
            let handlerResource: any = null;
            // Find extension spans in the dash0.lambda-extension scope
            const extensionSpans: any[] = [];
            let extensionResource: any = null;

            for (const rs of (spanPayload?.resourceSpans ?? [])) {
                for (const ss of (rs.scopeSpans ?? [])) {
                    if (ss.scope?.name === handlerScopeName) {
                        expect(ss.spans.length).toEqual(1);
                        handlerSpan = ss.spans[0];
                        handlerResource = rs.resource;
                    }
                    if (ss.scope?.name === 'dash0.lambda-extension') {
                        extensionSpans.push(...(ss.spans ?? []));
                        extensionResource = rs.resource;
                    }
                }
            }

            expect(handlerSpan, `Handler span not found in scope ${handlerScopeName}`).toBeDefined();

            // Root span: named after the function
            const rootSpan = extensionSpans.find((s: any) => s.name === functionName);
            expect(rootSpan, `Root span not found for ${functionName}`).toBeDefined();

            // Init span
            const initSpan = extensionSpans.find((s: any) => s.name === 'aws.lambda.initialization');
            expect(initSpan, 'Init span not found').toBeDefined();

            // Verify span kinds: root is SERVER (2), handler and init are INTERNAL (1)
            expect(rootSpan.kind).toEqual(2);
            expect(handlerSpan.kind).toEqual(1);
            expect(initSpan.kind).toEqual(1);

            // Verify parent-child relationships
            expect(handlerSpan.parentSpanId).toEqual(rootSpan.spanId);
            expect(initSpan.parentSpanId).toEqual(rootSpan.spanId);

            // Verify all share same traceId
            const traceId = rootSpan.traceId;
            expect(handlerSpan.traceId).toEqual(traceId);
            expect(initSpan.traceId).toEqual(traceId);

            // Verify faas.invocation_id on handler and root spans
            const handlerAttrs = getAttributesMap(handlerSpan.attributes);
            expect(handlerAttrs['faas.invocation_id'].stringValue).toEqual(invocationId);
            const rootAttrs = getAttributesMap(rootSpan.attributes);
            expect(rootAttrs['faas.invocation_id'].stringValue).toEqual(invocationId);

            // Verify faas.init_duration on root span
            expect(rootAttrs['faas.init_duration']).toBeDefined();

            // Check resource attributes on both
            checkResourceAttributes(handlerResource.attributes, functionName);
            checkResourceAttributes(extensionResource.attributes, functionName);

            // Verify service.name on both resources
            const handlerResAttrs = getAttributesMap(handlerResource.attributes);
            expect(handlerResAttrs['service.name'].stringValue).toEqual(functionName);
            const extensionResAttrs = getAttributesMap(extensionResource.attributes);
            expect(extensionResAttrs['service.name'].stringValue).toEqual(functionName);

            return {
                traceId,
                rootSpanId: rootSpan.spanId,
                handlerSpanId: handlerSpan.spanId,
                handlerSpan,
                resource: handlerResource,
            };
        } catch (error) {
            console.error(`Error fetching main spans on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
    throw new Error('checkMainSpans: exhausted all attempts');
}

export const checkOverheadSpan = async ({
    invocationId,
    functionName,
    traceId,
    rootSpanId,
}: {
    invocationId: string;
    functionName: string;
    traceId: string;
    rootSpanId: string;
}): Promise<void> => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        console.log(`Attempt ${attempt} to fetch overhead span for invocation ID ${invocationId}`);
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

            // Find overhead span in dash0.lambda-extension scope
            let overheadSpan: any = null;
            let overheadResource: any = null;
            for (const rs of (spanPayload?.resourceSpans ?? [])) {
                for (const ss of (rs.scopeSpans ?? [])) {
                    if (ss.scope?.name === 'dash0.lambda-extension') {
                        const found = (ss.spans ?? []).find((s: any) => s.name === 'aws.lambda.overhead');
                        if (found) {
                            overheadSpan = found;
                            overheadResource = rs.resource;
                        }
                    }
                }
            }

            expect(overheadSpan, 'Overhead span not found').toBeDefined();
            expect(overheadSpan.traceId).toEqual(traceId);
            expect(overheadSpan.parentSpanId).toEqual(rootSpanId);
            checkResourceAttributes(overheadResource.attributes, functionName);
            break;
        } catch (error) {
            console.error(`Error fetching overhead span on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) {
                throw error;
            }
        }
    }
}

export const checkMetrics = async ({
    functionName,
    metricNames,
}: {
    functionName: string;
    metricNames: string[];
}): Promise<void> => {
    for (const metricName of metricNames) {
        let found = false;
        for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
            await delay(RETRY_DELAY_MS);
            console.log(`Attempt ${attempt} to fetch metric ${metricName} for ${functionName}`);
            try {
                const params = new URLSearchParams({
                    dataset: DASH0_LAMBDA_TESTS_DATASET,
                    start: 'now-10m',
                    end: 'now',
                    step: '1m',
                    query: `{otel_metric_name = "${metricName}", otel_metric_type = "histogram", service_name = "${functionName}"}`,
                });
                const response = await fetch(DASH0_ENDPOINT + 'prometheus/api/v1/query_range', {
                    method: 'POST',
                    headers: {
                        authorization: `Bearer ${DASH0_TOKEN}`,
                        'content-type': 'application/x-www-form-urlencoded',
                    },
                    body: params.toString(),
                });

                const payload = await response.json() as any;
                expect(payload.status).toEqual('success');

                const results = payload.data?.result ?? [];
                const hasValue = results.some((r: any) =>
                    (r.values ?? []).some((v: any) => parseFloat(v[1]) >= 1)
                );

                if (hasValue) {
                    found = true;
                    break;
                }

                if (attempt === MAX_ATTEMPTS) {
                    throw new Error(`Metric ${metricName} for ${functionName} not found with value >= 1`);
                }
            } catch (error) {
                console.error(`Error fetching metric ${metricName} on attempt ${attempt}:`, error);
                if (attempt === MAX_ATTEMPTS) {
                    throw error;
                }
            }
        }
        expect(found, `Metric ${metricName} should have at least one value >= 1`).toBe(true);
    }
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
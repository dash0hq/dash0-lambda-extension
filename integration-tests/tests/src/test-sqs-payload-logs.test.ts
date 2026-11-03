import fetch from 'node-fetch';
import { setTimeout as delay } from 'node:timers/promises';
import { describe, expect, it } from 'vitest';
import { NODE_RUNTIMES } from '../../runtimes';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS, TEST_TIMEOUT_MS } from './config';
import { attributesToObject, getAttributesMap, getRequestPayload, invokeFunction, RESOURCE_PREFIX } from './utils';
import { fetchProducerSpans, getLambdaScopeName } from './utils-tracing-scenarios';

const fetchTelemetry = async (signal: 'spans' | 'logs', request: ReturnType<typeof getRequestPayload>) => {
    const response = await fetch(DASH0_ENDPOINT + signal, {
        method: 'POST',
        headers: {
            accept: 'application/json',
            authorization: `Bearer ${DASH0_TOKEN}`,
            'content-type': 'application/json',
        },
        body: JSON.stringify(request),
    });
    expect(response.ok).toBe(true);
    return await response.json() as any;
};

describe.concurrent('SQS send and receive payload logs', () => {
    for (const runtime of NODE_RUNTIMES) {
        for (const enabled of [true, false]) {
            const mode = enabled ? 'enabled' : 'disabled';
            it(`verifies sends and receives with payload logs ${mode} on ${runtime}`, async () => {
                const functionName = `${RESOURCE_PREFIX}tracing-sqs-payloads-${mode}-${runtime}`;
                const invocationId = await invokeFunction(functionName, true, false);
                // Supplementary spans and logs may flush on the next invocation.
                await delay(8000);
                await invokeFunction(functionName, true, false);
                const { traceId } = await fetchProducerSpans(invocationId, getLambdaScopeName(functionName));
                const logRequest = getRequestPayload(invocationId);
                const spanRequest = {
                    ...logRequest,
                    filter: [{ operator: 'is', key: 'service.name', value: functionName }],
                };

                const readAndCheck = async () => {
                    const [spanExport, logExport] = await Promise.all([
                        fetchTelemetry('spans', spanRequest),
                        fetchTelemetry('logs', logRequest),
                    ]);
                    const spans = (spanExport.resourceSpans ?? []).flatMap((rs: any) =>
                        rs.scopeSpans.flatMap((ss: any) => ss.spans)
                    ).filter((span: any) => span.traceId === traceId);
                    const operations = spans.filter((span: any) =>
                        ['SendMessage', 'SendMessageBatch', 'ReceiveMessage'].includes(getAttributesMap(span.attributes)['rpc.method']?.stringValue)
                    );
                    expect(operations.map((span: any) => getAttributesMap(span.attributes)['rpc.method'].stringValue).sort())
                        .toEqual(['ReceiveMessage', 'SendMessage', 'SendMessageBatch']);
                    for (const span of spans) {
                        expect(getAttributesMap(span.attributes)['messaging.publish.body']).toBeUndefined();
                        expect(getAttributesMap(span.attributes)['messaging.consume.body']).toBeUndefined();
                    }
                    const logs = (logExport.resourceLogs ?? []).flatMap((rl: any) =>
                        rl.scopeLogs.flatMap((sl: any) => sl.logRecords)
                    );
                    // Absence is meaningful only after real telemetry for this invocation arrived.
                    expect(logs.some((log: any) => log.body?.stringValue?.includes(`REPORT RequestId: ${invocationId}`))).toBe(true);
                    const payloads = logs.filter((log: any) =>
                        getAttributesMap(log.attributes ?? [])['dash0.faas.payload_type'] !== undefined
                    );
                    if (!enabled) {
                        expect(payloads).toHaveLength(0);
                        expect(logs.some((log: any) => log.body?.stringValue?.includes('SQS payload sends and receive completed'))).toBe(true);
                        return;
                    }

                    const messagingLogs = payloads.filter((log: any) =>
                        ['messaging_publish_body', 'messaging_consume_body'].includes(
                            getAttributesMap(log.attributes)['dash0.faas.payload_type'].stringValue
                        )
                    );
                    expect(messagingLogs).toHaveLength(3);
                    for (const span of operations) {
                        const matchingLogs = messagingLogs.filter((log: any) => log.spanId === span.spanId);
                        expect(matchingLogs).toHaveLength(1);
                        const log = matchingLogs[0];
                        expect(log.traceId).toBe(traceId);
                        const body = log.body?.stringValue
                            ? JSON.parse(log.body.stringValue)
                            : attributesToObject(log.attributes ?? []);
                        const method = getAttributesMap(span.attributes)['rpc.method'].stringValue;
                        expect(body).toMatchObject({
                            name: 'dash0_payload',
                            type: method === 'ReceiveMessage' ? 'messaging_consume_body' : 'messaging_publish_body',
                        });
                        if (method === 'ReceiveMessage') {
                            expect(body.message.Messages).toHaveLength(1);
                            expect(body.message.Messages[0].Body).toBe('****');
                        } else if (method === 'SendMessage') {
                            expect(body.message.QueueUrl.endsWith(`/${functionName}`)).toBe(true);
                            expect(body.message.MessageBody).toBe('****');
                        } else {
                            expect(body.message.QueueUrl.endsWith(`/${functionName}`)).toBe(true);
                            expect(body.message.Entries).toMatchObject([
                                { Id: 'order-a', MessageBody: '****' },
                                { Id: 'order-b', MessageBody: '****' },
                            ]);
                        }
                    }
                    expect(JSON.stringify(messagingLogs)).not.toContain('payload-single-secret');
                    expect(JSON.stringify(messagingLogs)).not.toContain('payload-batch-secret');
                };

                await expect.poll(readAndCheck, {
                    interval: RETRY_DELAY_MS,
                    timeout: MAX_ATTEMPTS * RETRY_DELAY_MS,
                }).toBeUndefined();
                if (!enabled) {
                    // Allow delayed ingestion after the REPORT before checking absence again.
                    // Do not retry this final check: an unexpected payload must fail the test.
                    await delay(2 * RETRY_DELAY_MS);
                    await readAndCheck();
                }
            }, TEST_TIMEOUT_MS);
        }
    }
});

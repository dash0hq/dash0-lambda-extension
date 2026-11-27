import { randomBytes } from 'crypto';
import { APIGatewayClient, GetRestApisCommand, RestApi } from '@aws-sdk/client-api-gateway';
import { setTimeout as delay } from 'timers/promises';
import fetch from 'node-fetch';
import { describe, expect, it } from 'vitest';
import { JAVA_RUNTIMES, NODE_RUNTIMES } from '../../runtimes';
import {
    checkLogs,
    checkMainSpans,
    findHandlerSpan,
    getRequestPayload,
    invokeFunction,
    LogToCheck,
    RESOURCE_PREFIX,
} from './utils';
import { DASH0_ENDPOINT, DASH0_TOKEN, MAX_ATTEMPTS, RETRY_DELAY_MS, TEST_TIMEOUT_MS } from './config';

// The functions run ~4 s with DASH0_FLUSH_INTERVAL=1000, so the extension sends
// their logs while they still run, before the handler span exists. Every log
// must still carry the IDs of the final span tree.

const NODE = NODE_RUNTIMES[NODE_RUNTIMES.length - 1];
const JAVA = JAVA_RUNTIMES[JAVA_RUNTIMES.length - 1];
const NODE_HANDLER_SCOPE = '@opentelemetry/instrumentation-aws-lambda';

// `userLogSeverity`: Node's console prefix carries INFO; Java's System.out has none.
const functionLogs = (userLogSeverity: string): LogToCheck[] => [
    { message: 'early-flush: before child span', severity: userLogSeverity },
    { message: 'early-flush: after child span', severity: userLogSeverity },
    { message: 'early-flush: done', severity: userLogSeverity },
    { message: 'START RequestId: ' },
    { message: 'END RequestId: ' },
    { message: JSON.stringify({ name: 'dash0_payload', type: 'lambda_event' }), isJson: true, attributes: { 'dash0.faas.payload_type': 'lambda_event' } },
];

const fetchHandlerSpan = async (invocationId: string, scopeName: string): Promise<any> => {
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
        await delay(RETRY_DELAY_MS);
        try {
            const response = await fetch(DASH0_ENDPOINT + 'spans', {
                method: 'POST',
                headers: {
                    accept: 'application/json',
                    authorization: `Bearer ${DASH0_TOKEN}`,
                    'content-type': 'application/json',
                },
                body: JSON.stringify(getRequestPayload(invocationId)),
            });
            return findHandlerSpan(await response.json(), scopeName).span;
        } catch (error) {
            console.error(`Error fetching handler span on attempt ${attempt}:`, error);
            if (attempt === MAX_ATTEMPTS) throw error;
        }
    }
};

const REGION = process.env.AWS_REGION ?? 'us-west-2';
const apiGateway = new APIGatewayClient({ region: REGION });

// Calls the function through its traced REST API until API Gateway samples a
// request (by default one per second, then 5%), and returns the invocation ID.
const invokeSampledThroughRestApi = async (restApiName: string): Promise<string> => {
    const apis: RestApi[] = [];
    let position: string | undefined;
    do {
        const page = await apiGateway.send(new GetRestApisCommand({ limit: 500, position }));
        apis.push(...(page.items ?? []));
        position = page.position;
    } while (position);
    const api = apis.find((a) => a.name === restApiName);
    expect(api, `REST API ${restApiName}`).toBeDefined();
    const url = `https://${api!.id}.execute-api.${REGION}.amazonaws.com/prod/`;

    for (let attempt = 1; attempt <= 10; attempt++) {
        const response = await fetch(url);
        expect(response.status).toEqual(200);
        const { requestId, traceHeader } = await response.json() as { requestId: string, traceHeader?: string };
        console.log(`${restApiName} attempt ${attempt}: ${requestId} ${traceHeader}`);
        if (traceHeader?.includes('Sampled=1')) {
            return requestId;
        }
        await delay(1500);
    }
    throw new Error(`API Gateway sampled none of 10 requests to ${restApiName}`);
};

describe.concurrent('Logs sent while the invocation runs', () => {
    it(`link to the root span in the upstream trace (${NODE})`, async () => {
        const functionName = `${RESOURCE_PREFIX}early-flush-${NODE}`;
        const upstreamTraceId = randomBytes(16).toString('hex');
        const upstreamSpanId = randomBytes(8).toString('hex');
        const payload = JSON.stringify({ headers: { traceparent: `00-${upstreamTraceId}-${upstreamSpanId}-01` } });
        const invocationId = await invokeFunction(functionName, true, false, payload);

        const { traceId, rootSpan, rootSpanId } = await checkMainSpans({
            invocationId,
            functionName,
            handlerScopeName: NODE_HANDLER_SCOPE,
        });
        // The handler continues the upstream trace, not the X-Ray trace from INVOKE.
        // The API returns IDs base64-encoded.
        const base64 = (hex: string) => Buffer.from(hex, 'hex').toString('base64');
        expect(traceId).toEqual(base64(upstreamTraceId));
        expect(rootSpan.parentSpanId).toEqual(base64(upstreamSpanId));

        await checkLogs({
            invocationId,
            functionName,
            traceId,
            parentSpanId: rootSpanId,
            success: true,
            logsToBeChecked: functionLogs('info'),
        });
    }, TEST_TIMEOUT_MS);

    it(`link to the Lambda function segment in X-Ray mode (${NODE})`, async () => {
        const functionName = `${RESOURCE_PREFIX}early-flush-xray-${NODE}`;
        const invocationId = await invokeFunction(functionName, true, false);

        // In X-Ray mode the handler's parent is the function segment, which
        // reaches Dash0 through X-Ray (see test-node-single-traced).
        const handlerSpan = await fetchHandlerSpan(invocationId, NODE_HANDLER_SCOPE);
        expect(handlerSpan.parentSpanId).toBeTruthy();

        await checkLogs({
            invocationId,
            functionName,
            traceId: handlerSpan.traceId,
            parentSpanId: handlerSpan.parentSpanId,
            success: true,
            logsToBeChecked: functionLogs('info'),
        });
    }, TEST_TIMEOUT_MS);

    it(`link to the handler span in X-Ray mode with PassThrough behind a traced REST API (${NODE})`, async () => {
        const functionName = `${RESOURCE_PREFIX}early-flush-xray-passthrough-${NODE}`;
        const invocationId = await invokeSampledThroughRestApi(`${RESOURCE_PREFIX}early-flush-xray-passthrough`);

        // Lambda records no function segment here. The handler's parent is
        // API Gateway's subsegment, which must not get the function's logs.
        const handlerSpan = await fetchHandlerSpan(invocationId, NODE_HANDLER_SCOPE);
        expect(handlerSpan.parentSpanId).toBeTruthy();

        await checkLogs({
            invocationId,
            functionName,
            traceId: handlerSpan.traceId,
            parentSpanId: handlerSpan.spanId,
            success: true,
            logsToBeChecked: functionLogs('info'),
        });
    }, TEST_TIMEOUT_MS);

    it(`link to the root span (${JAVA})`, async () => {
        const functionName = `${RESOURCE_PREFIX}early-flush-${JAVA}`;
        const invocationId = await invokeFunction(functionName, true, false);

        const { traceId, rootSpanId } = await checkMainSpans({
            invocationId,
            functionName,
            handlerScopeName: 'io.opentelemetry.aws-lambda-events-2.2',
        });

        await checkLogs({
            invocationId,
            functionName,
            traceId,
            parentSpanId: rootSpanId,
            success: true,
            logsToBeChecked: functionLogs(''),
        });
    }, TEST_TIMEOUT_MS);
});

import { describe, it } from 'vitest';
import {
    checkHttpSpan,
    checkLogs,
    checkMainSpans,
    checkMetrics,
    LogToCheck,
    invokeFunction,
} from './utils';
import { TEST_TIMEOUT_MS } from './config';

const commonLogs = (httpSpanId: string | undefined): LogToCheck[] => [
    { message: 'START RequestId: ' },
    { message: 'END RequestId: ' },
    { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true, attributes: { "dash0.faas.payload_type": "lambda_event" } },
    { message: JSON.stringify({ name: "dash0_payload", type: "http_request_body" }), isJson: true, spanId: httpSpanId, attributes: { "dash0.faas.payload_type": "http_request_body" } },
    { message: JSON.stringify({ name: "dash0_payload", type: "http_response_body" }), isJson: true, spanId: httpSpanId, attributes: { "dash0.faas.payload_type": "http_response_body" } },
];

const verifySanityInvocation = async (
    functionName: string,
    handlerScopeName: string,
    extraLogs: (httpSpanId: string | undefined) => LogToCheck[],
) => {
    const invocationId = await invokeFunction(functionName, true, false);

    const { traceId, rootSpanId, handlerSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName,
    });

    const httpSpanId = await checkHttpSpan({
        invocationId,
        functionName,
        traceId,
        parentSpanId: handlerSpanId,
    });

    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: true,
        logsToBeChecked: [...commonLogs(httpSpanId), ...extraLogs(httpSpanId)],
    });
    await invokeFunction(functionName, true, false);
    await checkMetrics({
        functionName,
        metricNames: ['faas.duration', 'dash0.faas.billed_duration', 'dash0.faas.memory_used', 'faas.init_duration'],
    });
};

describe('Production sanity checks', () => {
    it('Node.js lambda produces spans, logs, and metrics', async () => {
        await verifySanityInvocation(
            'sanity-node-success',
            '@opentelemetry/instrumentation-aws-lambda',
            () => [
                { message: 'Handler invoked with event:' },
                { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200 } }), isJson: true, attributes: { "dash0.faas.payload_type": "lambda_return_value" } },
            ],
        );
    }, TEST_TIMEOUT_MS);

    it('Python lambda produces spans, logs, and metrics', async () => {
        await verifySanityInvocation(
            'sanity-python-success',
            'opentelemetry.instrumentation.aws_lambda',
            () => [
                { message: 'response.status_code:' },
                { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200, body: '"Hello from Lambda!"' } }), isJson: true, attributes: { "dash0.faas.payload_type": "lambda_return_value" } },
            ],
        );
    }, TEST_TIMEOUT_MS);
});

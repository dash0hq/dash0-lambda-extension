import { describe, it } from 'vitest';
import { NODE_RUNTIMES } from '../../runtimes';
import {
    checkLogs,
    checkMainSpans,
    checkOverheadSpan,
    LogToCheck,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";
import {TEST_TIMEOUT_MS} from "./config";

const verifyCjsSuccess = async (functionName: string) => {
    const invocationPayload = JSON.stringify({ parameter1: 'right' });
    const invocationId = await invokeFunction(functionName, true, false, invocationPayload);

    const { traceId, rootSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: '@opentelemetry/instrumentation-aws-lambda',
    });

    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: true,
        logsToBeChecked: [
            { message: 'START RequestId: ' },
            { message: 'Handler invoked with event:' },
            { message: 'END RequestId: ' },
            { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
            { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200 } }), isJson: true },
        ],
    });

    // Overhead span is sent on the next invocation; trigger one
    await invokeFunction(functionName, true, false);
    await checkOverheadSpan({
        invocationId,
        functionName,
        traceId,
        rootSpanId,
    });
};

describe.concurrent('CJS-bundled Lambda invocation', () => {
    const runtimes = NODE_RUNTIMES;
    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}cjs-success-${runtime}`;
        it(
            `invokes ${functionName} successfully`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyCjsSuccess(functionName);
            },
            TEST_TIMEOUT_MS
        );
    }
});

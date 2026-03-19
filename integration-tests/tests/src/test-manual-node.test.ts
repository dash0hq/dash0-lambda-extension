import { describe, it } from 'vitest';
import {
    checkLogs,
    checkMainSpans,
    checkOverheadSpan,
    LogToCheck,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";

const verifyManualInstrumentation = async (functionName: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    const { traceId, rootSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: '@opentelemetry/instrumentation-aws-lambda',
    });

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: '[tracing] forceFlush complete' },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200 } }), isJson: true },
    ]
    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: true,
        logsToBeChecked,
    });

    // Overhead span is sent on the next invocation; trigger one
    await invokeFunction(functionName, true, false);
    await checkOverheadSpan({
        invocationId,
        functionName,
        traceId,
        rootSpanId,
    });
}

describe.concurrent('Manual instrumentation Lambda', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}manual-instrumentation-${runtime}`
        it(
            `invokes ${functionName} and receives trace`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyManualInstrumentation(functionName);
            },
            120_000
        );
    }
});

import { describe, it } from 'vitest';
import {
    checkLogs,
    checkMainSpans,
    checkOverheadSpan,
    LogToCheck,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";
import {TEST_TIMEOUT_MS} from "./config";


const verifyDockerizedInvocation = async (functionName: string, runtime: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    const scopeNameMap = {
        "python": "opentelemetry.instrumentation.aws_lambda",
        "node": "@opentelemetry/instrumentation-aws-lambda",
        "java": "io.opentelemetry.aws-lambda-events-2.2",
    };

    const { traceId, rootSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: scopeNameMap[runtime as keyof typeof scopeNameMap],
    });

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value" }), isJson: true },
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

describe.concurrent('Dockerized Lambda invocation', () => {
    const runtimes = ['python', 'node', 'java'];
    const architectures = ['x86_64', 'arm64'];

    for (const runtime of runtimes) {
        for (const architecture of architectures) {
            const functionName = `${RESOURCE_PREFIX}dockerized-${runtime}-${architecture}`;
            it(functionName, async () => {
                await verifyDockerizedInvocation(functionName, runtime);
            }, TEST_TIMEOUT_MS);
        }
    }
});

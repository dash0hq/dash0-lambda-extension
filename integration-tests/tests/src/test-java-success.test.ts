import { describe } from 'vitest';
import {
    checkLogs,
    checkMainSpans,
    checkOverheadSpan,
    LogToCheck,
    invokeFunction,
    runAllTests,
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, false);

    const handlerScopeName = traced ? "io.opentelemetry.aws-lambda-events-2.2" : "opentelemetry.instrumentation.aws_lambda";
    const { traceId, rootSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName,
    });

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: "Hello World from Java Lambda!" }), isJson: true },
    ]
    if (!invocationEnd) {
        logsToBeChecked.push({ message: 'REPORT RequestId: ' });
    }
    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: true,
        logsToBeChecked,
    });

    // Overhead span is sent on the next invocation; trigger one if needed
    if (invocationEnd) {
        await invokeFunction(functionName, true, false);
    }
    await checkOverheadSpan({
        invocationId,
        functionName,
        traceId,
        rootSpanId,
    });
}

describe.concurrent('Lambdainvocation', () => {
    const runtimes = ['java17', 'java21', 'java25'];
    runAllTests('success', runtimes, verifySuccessInvocation);
});

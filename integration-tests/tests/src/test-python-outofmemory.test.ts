import { describe } from 'vitest';
import {
    checkException,
    checkLogs,
    checkMainSpans,
    LogToCheck,
    invokeFunction,
    runAllTests,
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    const { traceId, rootSpanId, handlerSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
    });

    checkException(handlerSpan, 'Runtime.OutOfMemory');

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: "response.status_code:" },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
    ];
    if (!invocationEnd) {
        logsToBeChecked.push({ message: "Runtime.OutOfMemory" });
    }
    await checkLogs({
        invocationId,
        functionName,
        traceId: null,
        parentSpanId: null,
        success: false,
        logsToBeChecked,
    });
}

describe.concurrent('Lambda invocations with outofmemory', {retry: 1}, () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];
    runAllTests('outofmemory', runtimes, verifySuccessInvocation);
});

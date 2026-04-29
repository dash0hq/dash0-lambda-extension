import { describe } from 'vitest';
import { PYTHON_RUNTIMES } from '../../runtimes';
import {
    checkException,
    checkLogs,
    checkMainSpans,
    LogToCheck,
    invokeFunction,
    runAllTests,
} from "./utils";


// Python 3.14+ reports out-of-memory as `Runtime.ExitError` instead of
// `Runtime.OutOfMemory`. Older runtimes keep the `Runtime.OutOfMemory` label.
const expectedOomExceptionType = (functionName: string): string => {
    const match = functionName.match(/python3-(\d+)/);
    const minor = match ? parseInt(match[1], 10) : 0;
    return minor >= 14 ? 'Runtime.ExitError' : 'Runtime.OutOfMemory';
};

const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    const { traceId, rootSpanId, handlerSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
    });

    const exceptionType = expectedOomExceptionType(functionName);
    checkException(handlerSpan, exceptionType);

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: "response.status_code:" },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
    ];
    if (!invocationEnd) {
        logsToBeChecked.push({ message: exceptionType });
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
    const runtimes = PYTHON_RUNTIMES;
    runAllTests('outofmemory', runtimes, verifySuccessInvocation);
});

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


// Python 3.14's runtime is inconsistent about out-of-memory: it reports the
// exception as either `Runtime.OutOfMemory` or `Runtime.ExitError`. Older
// runtimes always use `Runtime.OutOfMemory`.
const acceptedOomExceptionTypes = (functionName: string): string[] => {
    const match = functionName.match(/python3-(\d+)/);
    const minor = match ? parseInt(match[1], 10) : 0;
    return minor >= 14
        ? ['Runtime.OutOfMemory', 'Runtime.ExitError']
        : ['Runtime.OutOfMemory'];
};

const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    const { traceId, rootSpanId, handlerSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
    });

    // checkException returns whichever accepted type the span actually reported,
    // so the log check below stays consistent with this invocation.
    const exceptionType = checkException(handlerSpan, acceptedOomExceptionTypes(functionName));

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

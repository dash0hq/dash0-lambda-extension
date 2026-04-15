import { describe } from 'vitest';
import { NODE_RUNTIMES } from '../../runtimes';
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

    checkException(handlerSpan, 'timeout');

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: "Handler invoked with event:" },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
    ];
    if (!invocationEnd) {
        logsToBeChecked.push({ message: "Status: timeout" });
    }
    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: false,
        logsToBeChecked,
    });
}

describe.concurrent('Lambda invocations with timeout', () => {
    const runtimes = NODE_RUNTIMES;
    runAllTests('timeout', runtimes, verifySuccessInvocation);
});

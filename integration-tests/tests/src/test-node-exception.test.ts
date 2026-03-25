import { describe, expect } from 'vitest';
import {
    checkLogs,
    checkMainSpans,
    checkMetrics,
    checkOverheadSpan,
    getAttributesMap,
    LogToCheck,
    invokeFunction,
    runAllTests,
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, true);

    const handlerScopeName = traced ? "@opentelemetry/instrumentation-aws-lambda" : "opentelemetry.instrumentation.aws_lambda";
    const { traceId, rootSpanId, handlerSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName,
    });

    // Check exception event (custom: status.message differs based on traced)
    const events = handlerSpan.events;
    expect(events.length).toEqual(1);
    const exceptionEvent = events[0];
    expect(exceptionEvent.name).toEqual('exception');
    const eventAttrMap = getAttributesMap(exceptionEvent.attributes);
    expect(eventAttrMap['exception.type'].stringValue).toEqual('ReferenceError');
    expect(handlerSpan.status.code).toEqual(2); // 2 = ERROR
    expect(handlerSpan.status.message).toEqual(traced ? 'nothing is not defined' : 'ReferenceError');

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: "Handler invoked with event:" },
        { message: "Invoke Error", severity: "error" },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value" }), isJson: true },
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
        await invokeFunction(functionName, true, true);
    }
    await checkOverheadSpan({
        invocationId,
        functionName,
        traceId,
        rootSpanId,
    });

    await checkMetrics({
        functionName,
        metricNames: ['faas.duration', 'dash0.faas.billed_duration', 'dash0.faas.memory_used', 'faas.init_duration'],
    });
}

describe.concurrent('Lambda invocation', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    runAllTests('exception', runtimes, verifySuccessInvocation);
});

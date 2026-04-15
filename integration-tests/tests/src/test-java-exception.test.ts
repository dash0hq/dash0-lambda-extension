import { describe, expect } from 'vitest';
import { JAVA_RUNTIMES } from '../../runtimes';
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
    const invocationId = await invokeFunction(functionName, invocationEnd, true, JSON.stringify({ parameter1: 'throw' }));

    const handlerScopeName = traced ? "io.opentelemetry.aws-lambda-events-2.2" : "opentelemetry.instrumentation.aws_lambda";
    const { traceId, rootSpanId, handlerSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName,
    });

    // Check exception event (custom: status.message and exception.message differ)
    const events = handlerSpan.events;
    expect(events.length).toEqual(1);
    const exceptionEvent = events[0];
    expect(exceptionEvent.name).toEqual('exception');
    const eventAttrMap = getAttributesMap(exceptionEvent.attributes);
    expect(eventAttrMap['exception.type'].stringValue).toEqual('java.lang.RuntimeException');
    expect(eventAttrMap['exception.message'].stringValue).toEqual("Intentional exception triggered by input 'throw'");
    expect(handlerSpan.status.code).toEqual(2); // 2 = ERROR
    expect(handlerSpan.status.message).toEqual(traced ? "" : 'java.lang.RuntimeException');

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: "Input received:" },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "throw" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value" }), isJson: true },
        { message: "java.lang.RuntimeException" },
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
        await invokeFunction(functionName, true, true, JSON.stringify({ parameter1: 'throw' }));
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
    const runtimes = JAVA_RUNTIMES;
    runAllTests('exception', runtimes, verifySuccessInvocation);
});

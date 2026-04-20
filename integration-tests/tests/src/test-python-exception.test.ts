import { describe, expect } from 'vitest';
import { PYTHON_RUNTIMES } from '../../runtimes';
import {
    checkHttpSpan,
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

    const { traceId, rootSpanId, handlerSpanId, handlerSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
    });

    // Check exception event (custom: status.message differs based on traced)
    const events = handlerSpan.events;
    expect(events.length).toEqual(1);
    const exceptionEvent = events[0];
    expect(exceptionEvent.name).toEqual('exception');
    const eventAttrMap = getAttributesMap(exceptionEvent.attributes);
    expect(eventAttrMap['exception.type'].stringValue).toEqual('KeyError');
    expect(handlerSpan.status.code).toEqual(2); // 2 = ERROR
    expect(handlerSpan.status.message).toEqual(traced ? '' : 'KeyError');

    let httpSpanId: string | undefined = undefined;
    if (traced) {
        httpSpanId = await checkHttpSpan({
            invocationId,
            functionName,
            traceId,
            parentSpanId: handlerSpanId,
        });
    }

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: "response.status_code:" },
        { message: "[ERROR] KeyError:" },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value" }), isJson: true },
    ]
    if (traced) {
        logsToBeChecked.push(
            { message: JSON.stringify({ name: "dash0_payload", type: "http_request_body", message: { title: "foo", body: "bar", userId: 1 } }), isJson: true, spanId: httpSpanId },
            { message: JSON.stringify({ name: "dash0_payload", type: "http_response_body" }), isJson: true, spanId: httpSpanId },
        );
    }
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
        metricNames: ['faas.invoke_duration', 'dash0.faas.billed_duration', 'faas.mem_usage', 'faas.init_duration'],
    });
}

describe.concurrent('Lambda invocation', () => {
    const runtimes = PYTHON_RUNTIMES;
    runAllTests('exception', runtimes, verifySuccessInvocation);
});

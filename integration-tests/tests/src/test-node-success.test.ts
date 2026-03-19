import { describe, expect } from 'vitest';
import {
    checkHttpSpan,
    checkLogs,
    checkMainSpans,
    checkOverheadSpan,
    getAttributesMap,
    LogToCheck,
    invokeFunction,
    runAllTests,
} from "./utils";


const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationPayload = JSON.stringify({ parameter1: 'right', masked_field: 'this should not be seen!' });
    const invocationId = await invokeFunction(functionName, invocationEnd, false, invocationPayload);

    const handlerScopeName = traced ? "@opentelemetry/instrumentation-aws-lambda" : "opentelemetry.instrumentation.aws_lambda";
    const { traceId, rootSpanId, handlerSpanId, resource } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName,
    });

    // Verify MASKED_FIELD is masked in resource attributes
    const resourceAttributes = getAttributesMap(resource.attributes);
    expect(resourceAttributes['process.environment_variable.MASKED_FIELD'].stringValue).toEqual('****');

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
        { message: "Handler invoked with event:" },
        { message: "let's parse this as a warning", severity: "warn" },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right", masked_field: "****" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200 } }), isJson: true },
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
        await invokeFunction(functionName, true, false);
    }
    await checkOverheadSpan({
        invocationId,
        functionName,
        traceId,
        rootSpanId,
    });
}

describe.concurrent('Lambda invocation', () => {
    const runtimes = ['nodejs20-x', 'nodejs22-x', 'nodejs24-x'];
    runAllTests('success', runtimes, verifySuccessInvocation);
});

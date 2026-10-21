import { describe, expect } from 'vitest';
import { JAVA_RUNTIMES } from '../../runtimes';
import {
    checkLogs,
    checkMainSpans,
    checkMetrics,
    checkOverheadSpan,
    checkHttpSpan,
    attributesToObject,
    LogToCheck,
    invokeFunction,
    runAllTests,
} from "./utils";


// The SDK records a captured header as a string array. Accept a plain or JSON-encoded string as
// well, in case the backend returns it that way.
const requestHeader = (attributes: Record<string, any>, name: string): unknown => {
    let value = attributes[`http.request.header.${name}`];
    if (typeof value === 'string' && value.startsWith('[')) {
        try { value = JSON.parse(value); } catch { /* keep the string */ }
    }
    return Array.isArray(value) ? value[0] : value;
};

const verifySuccessInvocation = async (functionName: string, invocationEnd: boolean, traced: boolean) => {
    const invocationId = await invokeFunction(functionName, invocationEnd, false);

    const handlerScopeName = traced ? "io.opentelemetry.aws-lambda-events-2.2" : "opentelemetry.instrumentation.aws_lambda";
    const { traceId, rootSpanId, handlerSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName,
    });

    let httpSpanId: string | undefined = undefined;
    if (traced) {
        httpSpanId = await checkHttpSpan({
            invocationId,
            functionName,
            traceId,
            parentSpanId: handlerSpanId,
            // The header is captured through OTEL_INSTRUMENTATION_HTTP_CLIENT_CAPTURE_REQUEST_HEADERS
            // and masked by the Java distro, because its name matches DASH0_MASK_RULES.
            checkSpan: (span) => {
                const httpAttributes = attributesToObject(span.attributes);
                const capturedHeaders = Object.keys(httpAttributes).filter((key) => key.startsWith('http.'));
                expect(requestHeader(httpAttributes, 'x-masked-field'), `http.* attributes: ${capturedHeaders.join(', ')}`).toEqual('****');
                expect(requestHeader(httpAttributes, 'content-type')).toEqual('application/json');
            },
        });
    }

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: 'END RequestId: ' },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: "Hello World from Java Lambda!" }), isJson: true },
    ]
    if (traced) {
        logsToBeChecked.push(
            // masked_field is masked by the Java distro, not the extension: the stack sets
            // DASH0_MASK_RULES, which the distro reads through its OpenTelemetry config.
            { message: JSON.stringify({ name: "dash0_payload", type: "http_request_body", message: { title: "foo", body: "bar", userId: 1, masked_field: "****" } }), isJson: true, spanId: httpSpanId, attributes: { "dash0.faas.payload_type": "http_request_body" } },
            // jsonplaceholder echoes the request, so the response body has the field too.
            { message: JSON.stringify({ name: "dash0_payload", type: "http_response_body", message: { title: "foo", masked_field: "****" } }), isJson: true, spanId: httpSpanId, attributes: { "dash0.faas.payload_type": "http_response_body" } },
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

    await checkMetrics({
        functionName,
        metricNames: ['faas.invoke_duration', 'dash0.faas.billed_duration', 'faas.mem_usage', 'faas.init_duration'],
    });
}

describe.concurrent('Lambdainvocation', () => {
    const runtimes = JAVA_RUNTIMES;
    runAllTests('success', runtimes, verifySuccessInvocation);
});

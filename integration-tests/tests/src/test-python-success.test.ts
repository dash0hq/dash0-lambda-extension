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
    const invocationId = await invokeFunction(functionName, invocationEnd, false);

    const { traceId, rootSpanId, handlerSpanId, resource } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
    });

    // Verify DASH0_TOKEN is masked in resource attributes
    const resourceAttributes = getAttributesMap(resource.attributes);
    if (!functionName.includes("python3-14")) {
        expect(resourceAttributes['process.environment_variable.DASH0_TOKEN'].stringValue).toEqual('****');
    }

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
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_event", message: { parameter1: "right" } }), isJson: true, attributes: { "dash0.faas.payload_type": "lambda_event" } },
        { message: JSON.stringify({ name: "dash0_payload", type: "lambda_return_value", message: { statusCode: 200,  body: '"Hello from Lambda!"' } }), isJson: true, attributes: { "dash0.faas.payload_type": "lambda_return_value" } },
    ]
    if (traced) {
        logsToBeChecked.push(
            { message: JSON.stringify({ name: "dash0_payload", type: "http_request_body", message: { title: "foo", userId: 1 } }), isJson: true, spanId: httpSpanId, attributes: { "dash0.faas.payload_type": "http_request_body" } },
            { message: JSON.stringify({ name: "dash0_payload", type: "http_response_body" }), isJson: true, spanId: httpSpanId, attributes: { "dash0.faas.payload_type": "http_response_body" } },
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
    const runtimes = PYTHON_RUNTIMES;
    runAllTests('success', runtimes, verifySuccessInvocation);
});

import { describe, it } from 'vitest';
import {
    checkHttpSpan,
    checkLogs,
    checkMainSpans,
    checkMetrics,
    checkOverheadSpan,
    LogToCheck,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";
import {TEST_TIMEOUT_MS} from "./config";

// Go runs on provided.al2023, where Lambda ignores AWS_LAMBDA_EXEC_WRAPPER: the function talks to
// the Runtime API directly, so the extension sees no event payload and relies on the handler span's
// own faas.invocation_id (iac/lambdas/go-manual).
const GO_OTELLAMBDA_SCOPE = 'go.opentelemetry.io/contrib/instrumentation/github.com/aws/aws-lambda-go/otellambda';

const verifyGoManualInstrumentation = async (functionName: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    const { traceId, rootSpanId, handlerSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: GO_OTELLAMBDA_SCOPE,
    });

    await checkHttpSpan({
        invocationId,
        functionName,
        traceId,
        parentSpanId: handlerSpanId,
    });

    const logsToBeChecked: LogToCheck[] = [
        { message: 'START RequestId: ' },
        { message: '[handler] outbound POST returned 201', severity: '' },
        { message: '[tracing] forceFlush complete', severity: '' },
        { message: 'END RequestId: ' },
    ];
    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: true,
        logsToBeChecked,
    });

    // Overhead span is sent on the next invocation; trigger one
    await invokeFunction(functionName, true, false);
    await checkOverheadSpan({
        invocationId,
        functionName,
        traceId,
        rootSpanId,
    });

    await checkMetrics({
        functionName,
        metricNames: ['dash0.test.go_manual_invocations'],
        metricType: '',
    });
}

describe.concurrent('Go manual instrumentation Lambda', () => {
    for (const architecture of ['x86_64', 'arm64']) {
        const functionName = `${RESOURCE_PREFIX}go-manual-instrumentation-${architecture}`;
        it(
            `invokes ${functionName} and receives trace, logs and metrics`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyGoManualInstrumentation(functionName);
            },
            TEST_TIMEOUT_MS
        );
    }
});

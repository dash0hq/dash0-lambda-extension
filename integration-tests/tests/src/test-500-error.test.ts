import { describe, expect, it } from 'vitest';
import {
    checkMainSpans,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";
import { TEST_TIMEOUT_MS } from "./config";

const verify500Error = async (functionName: string) => {
    const invocationId = await invokeFunction(functionName, true, false);

    const { handlerSpan, rootSpan } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: 'opentelemetry.instrumentation.aws_lambda',
    });

    // Handler span should have error status
    expect(handlerSpan.status.code).toEqual(2); // ERROR
    expect(handlerSpan.status.message).toEqual('Internal Server Error');

    // Root span should have error status
    expect(rootSpan.status.code).toEqual(2); // ERROR
    expect(rootSpan.status.message).toEqual('Internal Server Error');
};

describe.concurrent('Lambda 500 error', () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];

    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}apigw-error-500-${runtime}`;
        it(
            `returns error status for ${functionName}`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verify500Error(functionName);
            },
            TEST_TIMEOUT_MS,
        );
    }
});

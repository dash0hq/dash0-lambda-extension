import { describe, expect, it } from 'vitest';
import {
    checkMainSpans,
    getAttributesMap,
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
    expect(handlerSpan.status.message).toEqual('500');

    // Handler span should have an exception event with the right attributes
    expect(handlerSpan.events.length).toEqual(1);
    const exceptionEvent = handlerSpan.events[0];
    expect(exceptionEvent.name).toEqual('exception');
    const eventAttrs = getAttributesMap(exceptionEvent.attributes);
    expect(eventAttrs['exception.type'].stringValue).toEqual('500');
    expect(eventAttrs['exception.message'].stringValue).toEqual('Internal Server Error');
    expect(eventAttrs['exception.escaped'].stringValue).toEqual('False');

    // Root span should have error status
    expect(rootSpan.status.code).toEqual(2); // ERROR
    expect(rootSpan.status.message).toEqual('500');
};

describe.concurrent('Lambda 500 error', () => {
    const runtimes = ['python3-10', 'python3-11', 'python3-12', 'python3-13', 'python3-14'];

    for (const runtime of runtimes) {
        for (const traced of [true, false]) {
            const suffix = traced ? runtime : `untraced-${runtime}`;
            const functionName = `${RESOURCE_PREFIX}apigw-error-500-${suffix}`;
            it(
                `returns error status for ${functionName}`,
                async () => {
                    console.log(`Starting test for ${functionName}`, new Date().toISOString());
                    await verify500Error(functionName);
                },
                TEST_TIMEOUT_MS,
            );
        }
    }
});

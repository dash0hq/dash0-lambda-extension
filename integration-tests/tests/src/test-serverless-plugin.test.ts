import { describe, it } from 'vitest';
import {
    checkMainSpans,
    invokeFunction,
    RESOURCE_PREFIX,
} from "./utils";
import {TEST_TIMEOUT_MS} from "./config";

const SLS_FUNCTIONS = [
    { name: 'sls-3-test-python', handlerScopeName: 'opentelemetry.instrumentation.aws_lambda' },
    { name: 'sls-3-test-node',   handlerScopeName: '@opentelemetry/instrumentation-aws-lambda' },
    { name: 'sls-4-test-python', handlerScopeName: 'opentelemetry.instrumentation.aws_lambda' },
    { name: 'sls-4-test-node',   handlerScopeName: '@opentelemetry/instrumentation-aws-lambda' },
];

describe.concurrent('Serverless plugin integration', () => {
    for (const { name, handlerScopeName } of SLS_FUNCTIONS) {
        it(`${name}: should produce correct spans`, async () => {
            const functionName = `${RESOURCE_PREFIX}${name}`;
            const invocationId = await invokeFunction(functionName, true, false);

            await checkMainSpans({
                invocationId,
                functionName,
                handlerScopeName,
            });
        }, TEST_TIMEOUT_MS);
    }
});

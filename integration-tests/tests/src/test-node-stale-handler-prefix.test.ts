/*
 * The handler string carries a directory prefix the deployment package does not have:
 * the package is a single `index.js` at its root, the function is configured with
 * `dist/index.handler`. See `iac/lib/integration-tests-stack.ts` for the deployment and
 * `iac/lambdas/node-stale-handler-prefix/index.js` for the handler.
 *
 * The Lambda runtime loads the handler anyway. When none of `<taskRoot>/dist/index`,
 * `.js`, `.mjs` or `.cjs` exists it resolves `index` as a *bare* specifier, and NODE_PATH
 * on Lambda contains `/var/task`:
 *
 *   NODE_PATH=/opt/nodejs/node24/node_modules:/opt/nodejs/node_modules:
 *             /var/runtime/node_modules:/var/runtime:/var/task
 *
 * `@opentelemetry/instrumentation-aws-lambda` replicates only the three extensions, so on
 * its own it arms its require hook on `/var/task/dist/index` -- a path nothing ever loads.
 * The handler is silently left unwrapped. `opt/node/distro/src/lambdaHandlerResolution.ts` closes
 * that gap by working out what the runtime will actually load and passing it to upstream
 * as `lambdaHandler`.
 *
 * What makes this worth an integration test rather than only the unit-level reproduction
 * in `opt/node/test/handler-resolution.test.ts`: the whole failure mode depends on
 * NODE_PATH, which is set by the real Lambda runtime and by nothing else. A test that
 * constructs its own NODE_PATH is testing its own fixture.
 *
 * The discriminating assertion is `checkMainSpans` below. Without the fix the invocation
 * still succeeds and still logs -- every other assertion here passes -- and only the
 * handler span is missing.
 */

import { describe, it } from 'vitest';
import { NODE_RUNTIMES } from '../../runtimes';
import { checkLogs, checkMainSpans, invokeFunction, RESOURCE_PREFIX } from './utils';
import { TEST_TIMEOUT_MS } from './config';

const verifyStaleHandlerPrefix = async (functionName: string) => {
    const invocationPayload = JSON.stringify({ parameter1: 'right' });
    const invocationId = await invokeFunction(functionName, true, false, invocationPayload);

    // The handler span exists only because the handler path was corrected. Everything
    // else about this function looks healthy with or without that correction.
    const { traceId, rootSpanId } = await checkMainSpans({
        invocationId,
        functionName,
        handlerScopeName: '@opentelemetry/instrumentation-aws-lambda',
    });

    await checkLogs({
        invocationId,
        functionName,
        traceId,
        parentSpanId: rootSpanId,
        success: true,
        logsToBeChecked: [
            { message: 'START RequestId: ' },
            { message: 'Handler invoked with event:' },
            { message: 'END RequestId: ' },
        ],
    });
};

describe.concurrent('Lambda invocation with a stale directory prefix in the handler', () => {
    const runtimes = NODE_RUNTIMES;
    for (const runtime of runtimes) {
        const functionName = `${RESOURCE_PREFIX}stale-handler-prefix-${runtime}`;
        it(
            `produces a handler span for ${functionName}`,
            async () => {
                console.log(`Starting test for ${functionName}`, new Date().toISOString());
                await verifyStaleHandlerPrefix(functionName);
            },
            TEST_TIMEOUT_MS
        );
    }
});

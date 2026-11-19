import { describe, expect, it } from 'vitest';
import { setTimeout as delay } from 'timers/promises';
import { NODE_RUNTIMES } from '../../runtimes';
import { checkMainSpans, checkMetrics, getAttributesMap, invokeFunction, RESOURCE_PREFIX } from './utils';
import { TEST_TIMEOUT_MS } from './config';

// The function loads iac/lambdas/node/configure_hooks_extension.mjs before the layer. Its
// configureTracer adds a span processor that sets an attribute on every span, and its
// configureMeter adds a view that renames the handler's counter. Neither appears unless the layer
// built its providers from the configs the hooks returned.
describe.concurrent('Node configureTracer and configureMeter hooks', () => {
    for (const runtime of NODE_RUNTIMES) {
        const functionName = `${RESOURCE_PREFIX}configure-hooks-${runtime}`;
        it(
            `applies the hooks' configs in ${functionName}`,
            async () => {
                const invocationId = await invokeFunction(functionName, true, false);
                await delay(1000);
                await invokeFunction(functionName, true, false);

                const { handlerSpan } = await checkMainSpans({
                    invocationId,
                    functionName,
                    handlerScopeName: '@opentelemetry/instrumentation-aws-lambda',
                });
                expect(getAttributesMap(handlerSpan.attributes)['dash0.test.configure_tracer'].stringValue)
                    .toEqual('applied');

                await checkMetrics({
                    functionName,
                    metricNames: ['dash0.test.configure_meter_renamed'],
                    metricType: '',
                });
            },
            TEST_TIMEOUT_MS
        );
    }
});

import { describe, it } from 'vitest';
import { setTimeout as delay } from 'timers/promises';
import { NODE_RUNTIMES } from '../../runtimes';
import { checkMetrics, invokeFunction, RESOURCE_PREFIX } from './utils';
import { TEST_TIMEOUT_MS } from './config';

// Custom metrics are recorded through the plain OpenTelemetry API inside the handler
// (iac/lambdas/node/custom_metrics.ts). The handler neither creates a MeterProvider nor flushes;
// the metrics must arrive via the layer's global MeterProvider and its flush at handler exit.
// Only presence is asserted, not values.
const verifyCustomMetrics = async (functionName: string) => {
    await invokeFunction(functionName, true, false);
    await delay(1000);
    await invokeFunction(functionName, true, false);

    // The otel_metric_type label for a monotonic counter is not verified against live data, so
    // the type matcher is omitted for it; the metric name is unique to this test.
    await checkMetrics({
        functionName,
        metricNames: ['dash0.test.custom_counter'],
        metricType: '',
    });
    await checkMetrics({
        functionName,
        metricNames: ['dash0.test.custom_histogram'],
        metricType: 'histogram',
    });
};

describe.concurrent('Node custom metrics', () => {
    for (const runtime of NODE_RUNTIMES) {
        const functionName = `${RESOURCE_PREFIX}custommetrics-${runtime}`;
        it(
            `reports custom OpenTelemetry API metrics from ${functionName}`,
            async () => {
                await verifyCustomMetrics(functionName);
            },
            TEST_TIMEOUT_MS
        );
    }
});

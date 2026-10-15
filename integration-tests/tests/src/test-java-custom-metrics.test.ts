import { describe, it } from 'vitest';
import { setTimeout as delay } from 'timers/promises';
import { JAVA_RUNTIMES } from '../../runtimes';
import { checkMetrics, invokeFunction, RESOURCE_PREFIX } from './utils';
import { TEST_TIMEOUT_MS } from './config';

const ARCHITECTURES = ['x86_64', 'arm64'] as const;

// Custom metrics are recorded through the plain OpenTelemetry API inside the handler. There is no
// explicit flush in the handler; the metrics must arrive via the flush at handler exit.
// Values are cumulative, so only presence (>= 1) is asserted.
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

describe.concurrent('Java custom metrics', () => {
    for (const runtime of JAVA_RUNTIMES) {
        for (const architecture of ARCHITECTURES) {
            const functionName = `${RESOURCE_PREFIX}${runtime}-custommetrics-${architecture}`;
            it(
                `reports custom OpenTelemetry API metrics from ${functionName}`,
                async () => {
                    await verifyCustomMetrics(functionName);
                },
                TEST_TIMEOUT_MS
            );
        }
    }
});

// Records through the plain OpenTelemetry API. The span attribute and the metric name the test
// expects are added only by the hooks in configure_hooks_extension.mjs.
import { metrics } from '@opentelemetry/api';

const counter = metrics.getMeter('dash0-integration-test').createCounter('dash0.test.configure_meter_counter');

export async function handler() {
    counter.add(1);
    return { statusCode: 200 };
}

// What a customer writes to record custom metrics: only the OpenTelemetry API, bundled with the
// function. No MeterProvider setup and no forceFlush(); the layer registers the global
// MeterProvider and flushes it when the handler returns.
import { metrics } from '@opentelemetry/api';

const meter = metrics.getMeter('dash0-integration-test');
const counter = meter.createCounter('dash0.test.custom_counter', {
    description: 'Counts handler invocations',
});
const histogram = meter.createHistogram('dash0.test.custom_histogram', {
    description: 'Records a fixed value per invocation',
    unit: 'ms',
});

export async function handler(event: any) {
    console.log("Handler invoked with event:", event);
    counter.add(1, { 'test.attribute': 'value' });
    histogram.record(42, { 'test.attribute': 'value' });
    return {
        statusCode: 200,
        body: JSON.stringify({ message: "Success" }),
    };
}

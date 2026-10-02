package org.example;

import com.amazonaws.services.lambda.runtime.Context;
import com.amazonaws.services.lambda.runtime.RequestHandler;
import io.opentelemetry.api.GlobalOpenTelemetry;
import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.metrics.DoubleHistogram;
import io.opentelemetry.api.metrics.LongCounter;
import io.opentelemetry.api.metrics.Meter;

/**
 * Records custom metrics through the plain OpenTelemetry API. There is intentionally no explicit
 * flush or shutdown: the integration test proves that the instrumentation flushes at handler exit.
 */
public class CustomMetricsHandler implements RequestHandler<Object, String> {

    private static final Attributes ATTRIBUTES =
            Attributes.of(AttributeKey.stringKey("test.case"), "java-custom-metric");

    @Override
    public String handleRequest(Object input, Context context) {
        Meter meter = GlobalOpenTelemetry.getMeter("dash0.integration-tests.custom-metrics");

        LongCounter counter = meter.counterBuilder("dash0.test.custom_counter").setUnit("1").build();
        counter.add(1, ATTRIBUTES);

        DoubleHistogram histogram = meter.histogramBuilder("dash0.test.custom_histogram").setUnit("ms").build();
        histogram.record(42.0, ATTRIBUTES);

        return "custom metrics recorded";
    }
}

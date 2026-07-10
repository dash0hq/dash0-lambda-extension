'use strict';

console.log('[tracing] Loading tracing module');

const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
const { SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');
const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
const { resourceFromAttributes } = require('@opentelemetry/resources');
const { ATTR_SERVICE_NAME } = require('@opentelemetry/semantic-conventions');
const { AwsLambdaInstrumentation } = require('@opentelemetry/instrumentation-aws-lambda');
const { registerInstrumentations } = require('@opentelemetry/instrumentation');
const { MeterProvider, PeriodicExportingMetricReader } = require('@opentelemetry/sdk-metrics');
const { OTLPMetricExporter } = require('@opentelemetry/exporter-metrics-otlp-http');

const resource = resourceFromAttributes({
  [ATTR_SERVICE_NAME]: process.env.AWS_LAMBDA_FUNCTION_NAME || 'lambda-metrics',
});

// The extension accepts OTLP/HTTP on the default OpenTelemetry HTTP port (4318).
const exporter = new OTLPTraceExporter({
  url: `http://127.0.0.1:4318/v1/traces`,
});

const provider = new NodeTracerProvider({
  resource,
  spanProcessors: [new SimpleSpanProcessor(exporter)],
});

provider.register();

// Metrics setup
const metricExporter = new OTLPMetricExporter({
  url: `http://127.0.0.1:4318/v1/metrics`,
  headers: {
    Authorization: `Bearer ${process.env.DASH0_TOKEN}`,
  },
});

const meterProvider = new MeterProvider({
  resource,
  readers: [
    new PeriodicExportingMetricReader({
      exporter: metricExporter,
      exportIntervalMillis: 1000,
    }),
  ],
});

const meter = meterProvider.getMeter('lambda-metrics');

registerInstrumentations({
  instrumentations: [
    new AwsLambdaInstrumentation({
      disableAwsContextPropagation: true,
      responseHook: async (span) => {
        console.log('[tracing] responseHook called, flushing spans and metrics');
        await Promise.all([
          provider.forceFlush(),
          meterProvider.forceFlush(),
        ]);
        console.log('[tracing] forceFlush complete');
      },
    }),
  ],
});

module.exports = { provider, meterProvider, meter };

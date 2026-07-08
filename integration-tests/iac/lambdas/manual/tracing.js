'use strict';

console.log('[tracing] Loading tracing module');

const { NodeTracerProvider } = require('@opentelemetry/sdk-trace-node');
const { SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');
const { resourceFromAttributes } = require('@opentelemetry/resources');
const { ATTR_SERVICE_NAME } = require('@opentelemetry/semantic-conventions');
const { AwsLambdaInstrumentation } = require('@opentelemetry/instrumentation-aws-lambda');
const { registerInstrumentations } = require('@opentelemetry/instrumentation');
const { MeterProvider, PeriodicExportingMetricReader } = require('@opentelemetry/sdk-metrics');

// The extension accepts OTLP on the default OpenTelemetry ports:
// 4318 for OTLP/HTTP and 4317 for OTLP/gRPC.
const protocol = process.env.OTLP_PROTOCOL || 'http';
console.log(`[tracing] Using OTLP protocol: ${protocol}`);

let exporter;
let metricExporter;
if (protocol === 'grpc') {
  const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-grpc');
  const { OTLPMetricExporter } = require('@opentelemetry/exporter-metrics-otlp-grpc');
  exporter = new OTLPTraceExporter({
    url: `http://127.0.0.1:4317`,
  });
  metricExporter = new OTLPMetricExporter({
    url: `http://127.0.0.1:4317`,
    headers: {
      Authorization: `Bearer ${process.env.DASH0_TOKEN}`,
    },
  });
} else {
  const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
  const { OTLPMetricExporter } = require('@opentelemetry/exporter-metrics-otlp-http');
  exporter = new OTLPTraceExporter({
    url: `http://127.0.0.1:4318/v1/traces`,
  });
  metricExporter = new OTLPMetricExporter({
    url: `http://127.0.0.1:4318/v1/metrics`,
    headers: {
      Authorization: `Bearer ${process.env.DASH0_TOKEN}`,
    },
  });
}

const resource = resourceFromAttributes({
  [ATTR_SERVICE_NAME]: process.env.AWS_LAMBDA_FUNCTION_NAME || 'lambda-metrics',
});

const provider = new NodeTracerProvider({
  resource,
  spanProcessors: [new SimpleSpanProcessor(exporter)],
});

provider.register();

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

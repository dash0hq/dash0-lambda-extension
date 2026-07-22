'use strict';

// Stand-in for a customer's own in-app OpenTelemetry setup (Starling has one of
// these). Exporter endpoint and service name come from the standard OTEL_*
// environment variables:
//   OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:9009  -> Dash0 extension (local)
//   OTEL_EXPORTER_OTLP_ENDPOINT=https://ingress...     -> Dash0 cloud (direct)
//
// NOTE: @opentelemetry/api logs "Attempted duplicate registration of ..." as a
// diag error (it does not throw) when another SDK - e.g. the Dash0 distro
// loaded via NODE_OPTIONS - already registered the global providers. The
// second registration is silently ignored, so whichever SDK registers first
// owns tracing for the process.

const { diag, DiagConsoleLogger, DiagLogLevel } = require('@opentelemetry/api');
const { NodeSDK } = require('@opentelemetry/sdk-node');
const { SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');
const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
const { HttpInstrumentation } = require('@opentelemetry/instrumentation-http');
const { ExpressInstrumentation } = require('@opentelemetry/instrumentation-express');

// Surface OTel diagnostics (including the duplicate-registration errors) in
// CloudWatch logs.
diag.setLogger(new DiagConsoleLogger(), DiagLogLevel.WARN);

console.log('[app-otel] initializing in-app OpenTelemetry SDK');

// APP_OTEL_EXPORT_MODE=direct sends spans straight to the Dash0 ingress
// (bypassing the extension), reusing the extension's DASH0_* configuration.
// Default: honor the standard OTEL_EXPORTER_OTLP_* environment variables
// (e.g. pointing to the extension's local receiver on 127.0.0.1:9009).
const exporterConfig = {};
if ((process.env.APP_OTEL_EXPORT_MODE || '').toLowerCase() === 'direct') {
  exporterConfig.url = `${process.env.DASH0_ENDPOINT}/v1/traces`;
  exporterConfig.headers = { Authorization: `Bearer ${process.env.DASH0_TOKEN}` };
  if (process.env.DASH0_DATASET) {
    exporterConfig.headers['Dash0-Dataset'] = process.env.DASH0_DATASET;
  }
  console.log(`[app-otel] exporting directly to ${exporterConfig.url}`);
}

const sdk = new NodeSDK({
  // SimpleSpanProcessor exports every span immediately - important in Lambda,
  // where a batch processor may not get to flush before the sandbox freezes.
  spanProcessors: [new SimpleSpanProcessor(new OTLPTraceExporter(exporterConfig))],
  instrumentations: [new HttpInstrumentation(), new ExpressInstrumentation()],
});

try {
  sdk.start();
  console.log('[app-otel] in-app OpenTelemetry SDK started');
} catch (err) {
  console.error('[app-otel] in-app OpenTelemetry SDK failed to start', err);
}

process.on('SIGTERM', () => {
  sdk
    .shutdown()
    .catch((err) => console.error('[app-otel] shutdown failed', err))
    .finally(() => process.exit(0));
});

module.exports = { sdk };

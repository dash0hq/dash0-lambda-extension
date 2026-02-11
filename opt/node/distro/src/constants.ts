export const LUMIGO_LOGGING_NAMESPACE = '@lumigo/opentelemetry';

export const DEFAULT_LUMIGO_TRACES_ENDPOINT =
  'https://ga-otlp.lumigo-tracer-edge.golumigo.com/v1/traces';

// Since tracing is on by default, we allow omitting it and consider it enabled
export const TRACING_ENABLED =
  process.env.LUMIGO_ENABLE_TRACES === undefined ||
  process.env.LUMIGO_ENABLE_TRACES?.toLowerCase() === 'true';

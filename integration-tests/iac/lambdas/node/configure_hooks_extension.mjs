// What a customer ships next to the handler and loads with
// NODE_OPTIONS=--import /var/task/otel-extension.mjs, so it runs before the layer's /opt/init.mjs.
// No imports: the layer does not expose OpenTelemetry packages to user code.
globalThis.configureTracer = (defaultConfig) => ({
    ...defaultConfig,
    spanProcessors: [
        ...defaultConfig.spanProcessors,
        {
            onStart(span) {
                span.setAttribute('dash0.test.configure_tracer', 'applied');
            },
            onEnd() {},
            forceFlush: () => Promise.resolve(),
            shutdown: () => Promise.resolve(),
        },
    ],
});

globalThis.configureMeter = (defaultConfig) => ({
    ...defaultConfig,
    views: [
        ...(defaultConfig.views ?? []),
        { instrumentName: 'dash0.test.configure_meter_counter', name: 'dash0.test.configure_meter_renamed' },
    ],
});

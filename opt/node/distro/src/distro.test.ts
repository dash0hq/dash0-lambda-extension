const OTHER_DASH0_ENDPOINT =
  'http://ec2-34-215-6-94.us-west-2.compute.amazonaws.com:55681/v1/trace';
const DASH0_TOKEN = 't_10faa5e13e7844aaa1234';

// Reads the effective limit from a span instead of the provider's private config.
const attributeValueLengthLimitOf = (tracerProvider): number => {
  const span = tracerProvider
    .getTracer('span-limits-test')
    .startSpan('span-limits-test', { attributes: { probe: 'x'.repeat(10_000) } });
  return span.attributes.probe.length;
};

describe('Distro initialization', () => {
  const ORIGINAL_PROCESS_ENV = process.env;

  test.each(['v16.20.2', 'v17.9.1'])('skips initialization on unsupported Node.js %s', async (version) => {
    const originalVersion = process.version;
    const error = jest.spyOn(console, 'error').mockImplementation(() => {});
    Object.defineProperty(process, 'version', { value: version, configurable: true });
    try {
      await jest.isolateModulesAsync(async () => {
        const { init } = jest.requireActual('./distro');
        expect(await init).toBeUndefined();
      });
      expect(error).toHaveBeenCalledWith(expect.stringContaining('minimum supported version: 18.x'));
    } finally {
      Object.defineProperty(process, 'version', { value: originalVersion, configurable: true });
      error.mockRestore();
    }
  });

  afterAll(() => {
    process.env = ORIGINAL_PROCESS_ENV;
  });

  beforeEach(() => {
    /*
     * We have a limit on the size of env we sent to the backend, and the env
     * in the CI/CD goes over the limit, so the additional env vars we want to
     * check for scrubbing get dropped.
     */
    process.env = {};
  });

  afterEach(() => {
    process.env = {};

    // Unregister Otel globals
    jest.isolateModules(() => {
      const { context, diag, metrics, propagation, trace } = require('@opentelemetry/api');
      context.disable();
      metrics.disable();
      propagation.disable();
      trace.disable();
      diag.disable();
    });

    jest.resetAllMocks();
    jest.resetModules();
  });

  describe("with the 'DASH0_SWITCH_OFF' environment variable set to 'true'", () => {
    test('should not invoke trace initialization', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_SWITCH_OFF = 'true';

        const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
        jest.mock('@opentelemetry/exporter-trace-otlp-http');

        const { init } = jest.requireActual('./distro');
        const sdkInitialized = await init;

        expect(OTLPTraceExporter).not.toHaveBeenCalled();
        expect(sdkInitialized).toBeUndefined();
      });
    });
  });

  describe('with the DASH0_TOKEN environment variable set', () => {
    test('should initialize the OTLPTraceExporter', async () => {
      await jest.isolateModulesAsync(async () => {
        const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
        jest.mock('@opentelemetry/exporter-trace-otlp-http');

        process.env.DASH0_EXTENSION_ENDPOINT = OTHER_DASH0_ENDPOINT;
        process.env.DASH0_TOKEN = DASH0_TOKEN;

        const { init } = jest.requireActual('./distro');
        await init;

        expect(OTLPTraceExporter).toHaveBeenCalledWith({
          headers: {
            Authorization: 'Bearer t_10faa5e13e7844aaa1234',
          },
          url: OTHER_DASH0_ENDPOINT,
        });
      });
    });

  });

  describe('NodeTracerProvider should be initialize with span limit according to environment variables or default', () => {
    beforeEach(() => {
      process.env = { ...ORIGINAL_PROCESS_ENV };
    });

    test('NodeTracerProvider should be initialize with span limit equals to OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';
        process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT = '1';

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(attributeValueLengthLimitOf(tracerProvider)).toBe(1);
      });
    });

    test('NodeTracerProvider should be initialize with span limit equals to OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';
        process.env.OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT = '50';

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(attributeValueLengthLimitOf(tracerProvider)).toBe(50);
      });
    });

    test('NodeTracerProvider should be initialize with span limit equals to default value', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(attributeValueLengthLimitOf(tracerProvider)).toBe(4096);
      });
    });

    test('NodeTracerProvider should be initialize with span limit equals to OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT when both env. vars set', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';
        process.env.OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT = '50';
        process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT = '1';

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(attributeValueLengthLimitOf(tracerProvider)).toBe(1);
      });
    });
  });


  describe('with globalThis.configureTracer defined', () => {
    afterEach(() => {
      delete (globalThis as any).configureTracer;
    });

    test('builds the tracer provider from the config the hook returns', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT = '7';
        const { BatchSpanProcessor } = require('@opentelemetry/sdk-trace-base');
        let defaultProcessors: unknown[] = [];
        (globalThis as any).configureTracer = (defaultConfig) => {
          defaultProcessors = defaultConfig.spanProcessors;
          // Only the changed key: the other defaults must be merged back in.
          return {
            spanProcessors: [
              ...defaultConfig.spanProcessors,
              {
                onStart: (span) => span.setAttribute('custom', 'hook'),
                onEnd: () => {},
                forceFlush: () => Promise.resolve(),
                shutdown: () => Promise.resolve(),
              },
            ],
          };
        };

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(defaultProcessors).toEqual([expect.any(BatchSpanProcessor)]);
        const span = tracerProvider.getTracer('hook-test').startSpan('hook-test');
        expect(span.attributes.custom).toBe('hook');
        expect(span.resource.attributes['process.pid']).toBe(process.pid);
        expect(attributeValueLengthLimitOf(tracerProvider)).toBe(7);
      });
    });

    test.each([
      ['nothing', () => undefined],
      ['a Promise', async (defaultConfig) => defaultConfig],
    ])('keeps the default config and logs an error when the hook returns %s', async (_, hook) => {
      await jest.isolateModulesAsync(async () => {
        const error = jest.spyOn(console, 'error').mockImplementation(() => {});
        let defaultProcessor;
        (globalThis as any).configureTracer = (defaultConfig) => {
          [defaultProcessor] = defaultConfig.spanProcessors;
          jest.spyOn(defaultProcessor, 'onStart');
          return hook(defaultConfig);
        };

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;
        tracerProvider.getTracer('hook-test').startSpan('hook-test');

        expect(defaultProcessor.onStart).toHaveBeenCalled();
        expect(error).toHaveBeenCalledWith(expect.anything(), expect.stringContaining('globalThis.configureTracer must synchronously return'));
      });
    });
  });

  describe('with globalThis.configureMeter defined', () => {
    afterEach(() => {
      delete (globalThis as any).configureMeter;
    });

    test('builds the meter provider from the config the hook returns', async () => {
      await jest.isolateModulesAsync(async () => {
        const { MetricReader, PeriodicExportingMetricReader } = require('@opentelemetry/sdk-metrics');
        class TestReader extends MetricReader {
          protected onForceFlush = () => Promise.resolve();
          protected onShutdown = () => Promise.resolve();
        }
        const reader = new TestReader();
        let defaultReaders: unknown[] = [];
        (globalThis as any).configureMeter = (defaultConfig) => {
          defaultReaders = defaultConfig.readers;
          return { readers: [...defaultConfig.readers, reader] };
        };

        const { init } = jest.requireActual('./distro');
        const { meterProvider } = await init;
        meterProvider.getMeter('hook-test').createCounter('hook.counter').add(3);

        expect(defaultReaders).toEqual([expect.any(PeriodicExportingMetricReader)]);
        const { resourceMetrics } = await reader.collect();
        const [metric] = resourceMetrics.scopeMetrics[0].metrics;
        expect(metric.descriptor.name).toBe('hook.counter');
        expect(metric.dataPoints[0].value).toBe(3);
        expect(resourceMetrics.resource.attributes['process.pid']).toBe(process.pid);
        await meterProvider.shutdown();
      });
    });

    test.each([
      ['nothing', () => undefined],
      ['a Promise', async (defaultConfig) => defaultConfig],
    ])('keeps the default config and logs an error when the hook returns %s', async (_, hook) => {
      await jest.isolateModulesAsync(async () => {
        const error = jest.spyOn(console, 'error').mockImplementation(() => {});
        let defaultReader;
        (globalThis as any).configureMeter = (defaultConfig) => {
          [defaultReader] = defaultConfig.readers;
          return hook(defaultConfig);
        };

        const { init } = jest.requireActual('./distro');
        const { meterProvider } = await init;

        // collect() rejects unless the reader is registered with a meter provider.
        await expect(defaultReader.collect()).resolves.toBeDefined();
        expect(error).toHaveBeenCalledWith(expect.anything(), expect.stringContaining('globalThis.configureMeter must synchronously return'));
        await meterProvider.shutdown();
      });
    });
  });

  it('does not invoke console.error', async () => {
    console.error = jest.fn();

    await jest.isolateModulesAsync(async () => {
      const { init } = jest.requireActual('./distro');

      await init;

      expect(console.error).not.toHaveBeenCalled();
    });
  });
});

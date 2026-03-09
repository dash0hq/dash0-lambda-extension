const OTHER_DASH0_ENDPOINT =
  'http://ec2-34-215-6-94.us-west-2.compute.amazonaws.com:55681/v1/trace';
const DASH0_TOKEN = 't_10faa5e13e7844aaa1234';

describe('Distro initialization', () => {
  const ORIGINAL_PROCESS_ENV = process.env;

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
      const { context, diag, propagation, trace } = require('@opentelemetry/api');
      context.disable();
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

  describe('secret keys', () => {
    test('should be redacted by LUMIGO_SECRET_MASKING_REGEX from env vars', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';
        process.env.LUMIGO_SECRET_MASKING_REGEX = '["VAR_TO_MASK"]';
        process.env.VAR_TO_MASK = 'some value';

        const { init } = jest.requireActual('./distro');
        const { resource } = await init;

        const vars = JSON.parse(resource.attributes['process.environ']);
        expect(vars.VAR_TO_MASK).toEqual('****');
      });
    });

    describe.each(['LUMIGO_SECRET_MASKING_REGEX', 'LUMIGO_SECRET_MASKING_REGEX_ENVIRONMENT'])(
      'should be redacted entirely',
      (envVarName) => {
        test(`with the ${envVarName} set to 'all'`, async () => {
          await jest.isolateModulesAsync(async () => {
            process.env.DASH0_TOKEN = DASH0_TOKEN;
            process.env.OTEL_SERVICE_NAME = 'service-1';
            process.env[envVarName] = 'all';
            process.env.VAR_TO_MASK = 'some value';

            const { init } = jest.requireActual('./distro');
            const { resource } = await init;

            expect(JSON.parse(resource.attributes['process.environ'])).toEqual('****');
          });
        });
      }
    );

    test('should be redacted from env vars', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';
        process.env.AUTHORIZATION = 'some value';

        const { init } = jest.requireActual('./distro');
        const { resource } = await init;

        const vars = JSON.parse(resource.attributes['process.environ']);
        expect(vars.AUTHORIZATION).toEqual('****');
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

  describe('without the DASH0_TOKEN environment variable set', () => {
    test('should not initialize the OTLPTraceExporter', async () => {
      await jest.isolateModulesAsync(async () => {
        const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
        jest.mock('@opentelemetry/exporter-trace-otlp-http');

        const { init } = jest.requireActual('./distro');
        await init;

        expect(OTLPTraceExporter).not.toHaveBeenCalled();
      });
    });

    describe('with the DASH0_DEBUG_SPANDUMP variable set', () => {
      test('should initialize the FileSpanExporter', async () => {
        await jest.isolateModulesAsync(async () => {
          process.env.DASH0_DEBUG_SPANDUMP = '/dev/stdout';

          jest.mock('./exporters');

          const { init } = jest.requireActual('./distro');
          await init;

          const { FileSpanExporter } = require('./exporters');
          expect(FileSpanExporter).toHaveBeenCalledWith('/dev/stdout');
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

        expect(tracerProvider._config.spanLimits['attributeValueLengthLimit']).toBe(1);
      });
    });

    test('NodeTracerProvider should be initialize with span limit equals to OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';
        process.env.OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT = '50';

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(tracerProvider._config.spanLimits['attributeValueLengthLimit']).toBe(50);
      });
    });

    test('NodeTracerProvider should be initialize with span limit equals to default value', async () => {
      await jest.isolateModulesAsync(async () => {
        process.env.DASH0_TOKEN = DASH0_TOKEN;
        process.env.OTEL_SERVICE_NAME = 'service-1';

        const { init } = jest.requireActual('./distro');
        const { tracerProvider } = await init;

        expect(tracerProvider._config.spanLimits['attributeValueLengthLimit']).toBe(2097152);
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

        expect(tracerProvider._config.spanLimits['attributeValueLengthLimit']).toBe(1);
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

// eslint-disable-next-line @typescript-eslint/no-var-requires
const ServerlessDash0Plugin = require('../src/index');

function createMockServerless(overrides: {
  provider?: Record<string, unknown>;
  dash0?: Record<string, unknown>;
  functions?: Record<string, Record<string, unknown>>;
} = {}): any {
  return {
    cli: { log: jest.fn() },
    getProvider: jest.fn().mockReturnValue({
      request: jest.fn(),
    }),
    service: {
      provider: { region: 'us-east-1', runtime: 'nodejs20.x', ...overrides.provider },
      custom: {
        dash0: {
          endpoint: 'https://ingress.eu-west-1.aws.dash0.com:4318',
          token: 'test-token',
          layerVersion: 42,
          ...overrides.dash0,
        },
      },
      functions: overrides.functions || {},
    },
  };
}

describe('ServerlessDash0Plugin', () => {
  it('adds layer and env vars to a traced function', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    const func = serverless.service.functions.myFunc;
    expect(func.layers).toEqual([
      'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node:42',
    ]);
    expect(func.environment.AWS_LAMBDA_EXEC_WRAPPER).toBe('/opt/wrapper');
    expect(func.environment.DASH0_ENDPOINT).toBe('https://ingress.eu-west-1.aws.dash0.com:4318');
    expect(func.environment.DASH0_TOKEN).toBe('test-token');
  });

  it('does not touch untraced functions', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x' },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    const func = serverless.service.functions.myFunc;
    expect(func.layers).toBeUndefined();
    expect(func.environment).toBeUndefined();
  });

  it('does not touch functions with dash0-traced: false', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': false },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toBeUndefined();
  });

  it('accepts dash0-traced as string "true"', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': 'true' },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toHaveLength(1);
  });

  it('preserves existing layers', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: {
          handler: 'handler.hello',
          runtime: 'nodejs20.x',
          'dash0-traced': true,
          layers: ['arn:aws:lambda:us-east-1:123456:layer:my-layer:1'],
        },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    const func = serverless.service.functions.myFunc;
    expect(func.layers).toHaveLength(2);
    expect(func.layers![0]).toBe('arn:aws:lambda:us-east-1:123456:layer:my-layer:1');
    expect(func.layers![1]).toContain('dash0-extension-node');
  });

  it('does not duplicate layer ARN on repeated invocation', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toHaveLength(1);
  });

  it('does not overwrite existing env vars', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: {
          handler: 'handler.hello',
          runtime: 'nodejs20.x',
          'dash0-traced': true,
          environment: { DASH0_EXTENSION_LOG_LEVEL: 'debug' },
        },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.environment.DASH0_EXTENSION_LOG_LEVEL).toBe('debug');
    expect(serverless.service.functions.myFunc.environment.AWS_LAMBDA_EXEC_WRAPPER).toBe('/opt/wrapper');
  });

  it('uses correct layer for each runtime', async () => {
    const serverless = createMockServerless({
      functions: {
        nodeFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
        pyFunc: { handler: 'handler.hello', runtime: 'python3.12', 'dash0-traced': true },
        javaFunc: { handler: 'handler.hello', runtime: 'java21', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.nodeFunc.layers![0]).toContain('dash0-extension-node');
    expect(serverless.service.functions.pyFunc.layers![0]).toContain('dash0-extension-python');
    expect(serverless.service.functions.javaFunc.layers![0]).toContain('dash0-extension-java');
  });

  it('falls back to provider runtime', async () => {
    const serverless = createMockServerless({
      provider: { runtime: 'python3.12' },
      functions: {
        myFunc: { handler: 'handler.hello', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers![0]).toContain('dash0-extension-python');
  });

  it('warns and skips unsupported runtime', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'ruby3.3', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toBeUndefined();
    expect(serverless.cli.log).toHaveBeenCalledWith(
      expect.stringContaining('unsupported runtime')
    );
  });

  it('warns and skips container image functions', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { image: 'my-image:latest', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toBeUndefined();
    expect(serverless.cli.log).toHaveBeenCalledWith(
      expect.stringContaining('container image functions')
    );
  });

  it('throws on missing config', async () => {
    const serverless = createMockServerless();
    serverless.service.custom.dash0 = undefined as unknown;

    const plugin = new ServerlessDash0Plugin(serverless, {});
    await expect(plugin.hooks['before:package:initialize']()).rejects.toThrow('[serverless-dash0]');
  });

  it('uses region from options over provider', async () => {
    const serverless = createMockServerless({
      provider: { region: 'us-east-1' },
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, { region: 'eu-west-1' });
    await plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers![0]).toContain('eu-west-1');
  });

  it('works with SLS v4 logging utils', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const logUtils = { log: { notice: jest.fn(), warning: jest.fn(), error: jest.fn() } };
    const plugin = new ServerlessDash0Plugin(serverless, {}, logUtils);
    await plugin.hooks['before:package:initialize']();

    expect(logUtils.log.notice).toHaveBeenCalledWith(
      expect.stringContaining('Configured tracing')
    );
  });

  it('uses SLS v4 warning for unsupported runtime', async () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'ruby3.3', 'dash0-traced': true },
      },
    });

    const logUtils = { log: { notice: jest.fn(), warning: jest.fn(), error: jest.fn() } };
    const plugin = new ServerlessDash0Plugin(serverless, {}, logUtils);
    await plugin.hooks['before:package:initialize']();

    expect(logUtils.log.warning).toHaveBeenCalledWith(
      expect.stringContaining('unsupported runtime')
    );
  });

  describe('layerAccountId', () => {
    it('uses custom account ID in layer ARN', async () => {
      const serverless = createMockServerless({
        dash0: { layerAccountId: '999888777666' },
        functions: {
          myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
        },
      });

      const plugin = new ServerlessDash0Plugin(serverless, {});
      await plugin.hooks['before:package:initialize']();

      expect(serverless.service.functions.myFunc.layers![0]).toBe(
        'arn:aws:lambda:us-east-1:999888777666:layer:dash0-extension-node:42'
      );
    });
  });

  describe('layerVersion: "latest"', () => {
    it('resolves latest version via AWS API', async () => {
      const serverless = createMockServerless({
        dash0: { layerVersion: 'latest' },
        functions: {
          myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
        },
      });

      const provider = serverless.getProvider('aws');
      provider.request.mockResolvedValue({
        LayerVersions: [{ Version: 7 }],
      });

      const plugin = new ServerlessDash0Plugin(serverless, {});
      await plugin.hooks['before:package:initialize']();

      expect(provider.request).toHaveBeenCalledWith('Lambda', 'listLayerVersions', {
        LayerName: 'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node',
        MaxItems: 1,
      });
      expect(serverless.service.functions.myFunc.layers![0]).toBe(
        'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node:7'
      );
    });

    it('caches latest version across functions with same layer', async () => {
      const serverless = createMockServerless({
        dash0: { layerVersion: 'latest' },
        functions: {
          func1: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
          func2: { handler: 'handler.hello', runtime: 'nodejs22.x', 'dash0-traced': true },
        },
      });

      const provider = serverless.getProvider('aws');
      provider.request.mockResolvedValue({
        LayerVersions: [{ Version: 5 }],
      });

      const plugin = new ServerlessDash0Plugin(serverless, {});
      await plugin.hooks['before:package:initialize']();

      // Only one API call for both node functions
      expect(provider.request).toHaveBeenCalledTimes(1);
    });

    it('makes separate API calls for different runtimes', async () => {
      const serverless = createMockServerless({
        dash0: { layerVersion: 'latest' },
        functions: {
          nodeFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
          pyFunc: { handler: 'handler.hello', runtime: 'python3.12', 'dash0-traced': true },
        },
      });

      const provider = serverless.getProvider('aws');
      provider.request
        .mockResolvedValueOnce({ LayerVersions: [{ Version: 10 }] })
        .mockResolvedValueOnce({ LayerVersions: [{ Version: 8 }] });

      const plugin = new ServerlessDash0Plugin(serverless, {});
      await plugin.hooks['before:package:initialize']();

      expect(provider.request).toHaveBeenCalledTimes(2);
      expect(serverless.service.functions.nodeFunc.layers![0]).toContain(':10');
      expect(serverless.service.functions.pyFunc.layers![0]).toContain(':8');
    });

    it('uses custom account ID when resolving latest', async () => {
      const serverless = createMockServerless({
        dash0: { layerVersion: 'latest', layerAccountId: '999888777666' },
        functions: {
          myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
        },
      });

      const provider = serverless.getProvider('aws');
      provider.request.mockResolvedValue({
        LayerVersions: [{ Version: 3 }],
      });

      const plugin = new ServerlessDash0Plugin(serverless, {});
      await plugin.hooks['before:package:initialize']();

      expect(provider.request).toHaveBeenCalledWith('Lambda', 'listLayerVersions', {
        LayerName: 'arn:aws:lambda:us-east-1:999888777666:layer:dash0-extension-node',
        MaxItems: 1,
      });
      expect(serverless.service.functions.myFunc.layers![0]).toBe(
        'arn:aws:lambda:us-east-1:999888777666:layer:dash0-extension-node:3'
      );
    });

    it('throws if no versions found', async () => {
      const serverless = createMockServerless({
        dash0: { layerVersion: 'latest' },
        functions: {
          myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
        },
      });

      const provider = serverless.getProvider('aws');
      provider.request.mockResolvedValue({ LayerVersions: [] });

      const plugin = new ServerlessDash0Plugin(serverless, {});
      await expect(plugin.hooks['before:package:initialize']()).rejects.toThrow('No versions found');
    });
  });
});

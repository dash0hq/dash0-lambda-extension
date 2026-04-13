// eslint-disable-next-line @typescript-eslint/no-var-requires
const ServerlessDash0Plugin = require('../src/index');

function createMockServerless(overrides: {
  provider?: Record<string, unknown>;
  dash0?: Record<string, unknown>;
  functions?: Record<string, Record<string, unknown>>;
} = {}): any {
  return {
    cli: { log: jest.fn() },
    getProvider: jest.fn().mockReturnValue({}),
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
  it('adds layer and env vars to a traced function', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    const func = serverless.service.functions.myFunc;
    expect(func.layers).toEqual([
      'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node:42',
    ]);
    expect(func.environment.AWS_LAMBDA_EXEC_WRAPPER).toBe('/opt/wrapper');
    expect(func.environment.DASH0_ENDPOINT).toBe('https://ingress.eu-west-1.aws.dash0.com:4318');
    expect(func.environment.DASH0_TOKEN).toBe('test-token');
  });

  it('does not touch untraced functions', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x' },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    const func = serverless.service.functions.myFunc;
    expect(func.layers).toBeUndefined();
    expect(func.environment).toBeUndefined();
  });

  it('does not touch functions with dash0-traced: false', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': false },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toBeUndefined();
  });

  it('accepts dash0-traced as string "true"', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': 'true' },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toHaveLength(1);
  });

  it('preserves existing layers', () => {
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
    plugin.hooks['before:package:initialize']();

    const func = serverless.service.functions.myFunc;
    expect(func.layers).toHaveLength(2);
    expect(func.layers![0]).toBe('arn:aws:lambda:us-east-1:123456:layer:my-layer:1');
    expect(func.layers![1]).toContain('dash0-extension-node');
  });

  it('does not duplicate layer ARN on repeated invocation', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toHaveLength(1);
  });

  it('does not overwrite existing env vars', () => {
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
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.environment.DASH0_EXTENSION_LOG_LEVEL).toBe('debug');
    expect(serverless.service.functions.myFunc.environment.AWS_LAMBDA_EXEC_WRAPPER).toBe('/opt/wrapper');
  });

  it('uses correct layer for each runtime', () => {
    const serverless = createMockServerless({
      functions: {
        nodeFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
        pyFunc: { handler: 'handler.hello', runtime: 'python3.12', 'dash0-traced': true },
        javaFunc: { handler: 'handler.hello', runtime: 'java21', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.nodeFunc.layers![0]).toContain('dash0-extension-node');
    expect(serverless.service.functions.pyFunc.layers![0]).toContain('dash0-extension-python');
    expect(serverless.service.functions.javaFunc.layers![0]).toContain('dash0-extension-java');
  });

  it('falls back to provider runtime', () => {
    const serverless = createMockServerless({
      provider: { runtime: 'python3.12' },
      functions: {
        myFunc: { handler: 'handler.hello', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers![0]).toContain('dash0-extension-python');
  });

  it('warns and skips unsupported runtime', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'ruby3.3', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toBeUndefined();
    expect(serverless.cli.log).toHaveBeenCalledWith(
      expect.stringContaining('unsupported runtime')
    );
  });

  it('warns and skips container image functions', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { image: 'my-image:latest', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, {});
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers).toBeUndefined();
    expect(serverless.cli.log).toHaveBeenCalledWith(
      expect.stringContaining('container image functions')
    );
  });

  it('throws on missing config', () => {
    const serverless = createMockServerless();
    serverless.service.custom.dash0 = undefined as unknown;

    const plugin = new ServerlessDash0Plugin(serverless, {});
    expect(() => plugin.hooks['before:package:initialize']()).toThrow('[serverless-dash0]');
  });

  it('uses region from options over provider', () => {
    const serverless = createMockServerless({
      provider: { region: 'us-east-1' },
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const plugin = new ServerlessDash0Plugin(serverless, { region: 'eu-west-1' });
    plugin.hooks['before:package:initialize']();

    expect(serverless.service.functions.myFunc.layers![0]).toContain('eu-west-1');
  });

  it('works with SLS v4 logging utils', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'nodejs20.x', 'dash0-traced': true },
      },
    });

    const logUtils = { log: { notice: jest.fn(), warning: jest.fn(), error: jest.fn() } };
    const plugin = new ServerlessDash0Plugin(serverless, {}, logUtils);
    plugin.hooks['before:package:initialize']();

    expect(logUtils.log.notice).toHaveBeenCalledWith(
      expect.stringContaining('Configured tracing')
    );
  });

  it('uses SLS v4 warning for unsupported runtime', () => {
    const serverless = createMockServerless({
      functions: {
        myFunc: { handler: 'handler.hello', runtime: 'ruby3.3', 'dash0-traced': true },
      },
    });

    const logUtils = { log: { notice: jest.fn(), warning: jest.fn(), error: jest.fn() } };
    const plugin = new ServerlessDash0Plugin(serverless, {}, logUtils);
    plugin.hooks['before:package:initialize']();

    expect(logUtils.log.warning).toHaveBeenCalledWith(
      expect.stringContaining('unsupported runtime')
    );
  });
});

import { buildEnvironment } from '../src/env-injector';
import { Dash0Config } from '../src/config-validator';

describe('buildEnvironment', () => {
  const minimalConfig: Dash0Config = {
    endpoint: 'https://ingress.eu-west-1.aws.dash0.com:4318',
    token: 'test-token',
    layerVersion: 42,
  };

  it('sets required env vars with token', () => {
    const env = buildEnvironment(minimalConfig);
    expect(env.AWS_LAMBDA_EXEC_WRAPPER).toBe('/opt/wrapper');
    expect(env.DASH0_ENDPOINT).toBe('https://ingress.eu-west-1.aws.dash0.com:4318');
    expect(env.DASH0_TOKEN).toBe('test-token');
    expect(env.DASH0_TOKEN_SECRET_ARN).toBeUndefined();
  });

  it('sets DASH0_TOKEN_SECRET_ARN when provided', () => {
    const config: Dash0Config = {
      ...minimalConfig,
      token: undefined,
      tokenSecretArn: 'arn:aws:secretsmanager:us-east-1:123:secret:foo',
    };
    const env = buildEnvironment(config);
    expect(env.DASH0_TOKEN).toBeUndefined();
    expect(env.DASH0_TOKEN_SECRET_ARN).toBe('arn:aws:secretsmanager:us-east-1:123:secret:foo');
  });

  it('sets both token and tokenSecretArn when both provided', () => {
    const config: Dash0Config = {
      ...minimalConfig,
      tokenSecretArn: 'arn:aws:secretsmanager:us-east-1:123:secret:foo',
    };
    const env = buildEnvironment(config);
    expect(env.DASH0_TOKEN).toBe('test-token');
    expect(env.DASH0_TOKEN_SECRET_ARN).toBe('arn:aws:secretsmanager:us-east-1:123:secret:foo');
  });

  it('sets tokenSecretKey when provided', () => {
    const config: Dash0Config = {
      ...minimalConfig,
      tokenSecretArn: 'arn:...',
      tokenSecretKey: 'apiToken',
    };
    const env = buildEnvironment(config);
    expect(env.DASH0_TOKEN_SECRET_KEY).toBe('apiToken');
  });

  it('sets optional env vars when configured', () => {
    const config: Dash0Config = {
      ...minimalConfig,
      dataset: 'my-dataset',
      extensionLogLevel: 'debug',
      disableAutoInstrumentation: true,
      sendOnInvocationEnd: false,
      requestTimeout: 5000,
      xrayTracesEnabled: true,
      maskRules: '[".*secret.*"]',
    };
    const env = buildEnvironment(config);
    expect(env.DASH0_DATASET).toBe('my-dataset');
    expect(env.DASH0_EXTENSION_LOG_LEVEL).toBe('debug');
    expect(env.DASH0_DISABLE_AUTO_INSTRUMENTATION).toBe('true');
    expect(env.DASH0_SEND_ON_INVOCATION_END).toBe('false');
    expect(env.DASH0_REQUEST_TIMEOUT).toBe('5000');
    expect(env.DASH0_XRAY_TRACES_ENABLED).toBe('true');
    expect(env.DASH0_MASK_RULES).toBe('[".*secret.*"]');
  });

  it('omits optional env vars when not configured', () => {
    const env = buildEnvironment(minimalConfig);
    expect(env.DASH0_DATASET).toBeUndefined();
    expect(env.DASH0_EXTENSION_LOG_LEVEL).toBeUndefined();
    expect(env.DASH0_DISABLE_AUTO_INSTRUMENTATION).toBeUndefined();
  });
});

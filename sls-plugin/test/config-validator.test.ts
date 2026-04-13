import { validateConfig } from '../src/config-validator';

const VALID_CONFIG = {
  endpoint: 'https://ingress.eu-west-1.aws.dash0.com:4318',
  token: 'test-token',
  layerVersion: 42,
};

describe('validateConfig', () => {
  it('passes with valid minimal config (token)', () => {
    expect(() => validateConfig(VALID_CONFIG)).not.toThrow();
  });

  it('passes with valid config (tokenSecretArn)', () => {
    const config = { ...VALID_CONFIG, token: undefined, tokenSecretArn: 'arn:aws:secretsmanager:us-east-1:123:secret:foo' };
    expect(() => validateConfig(config)).not.toThrow();
  });

  it('passes with both token and tokenSecretArn', () => {
    const config = { ...VALID_CONFIG, tokenSecretArn: 'arn:aws:secretsmanager:us-east-1:123:secret:foo' };
    expect(() => validateConfig(config)).not.toThrow();
  });

  it('returns the validated config', () => {
    const result = validateConfig(VALID_CONFIG);
    expect(result.endpoint).toBe(VALID_CONFIG.endpoint);
    expect(result.layerVersion).toBe(42);
  });

  it('throws if config is null', () => {
    expect(() => validateConfig(null)).toThrow('[serverless-dash0] Missing "custom.dash0"');
  });

  it('throws if config is undefined', () => {
    expect(() => validateConfig(undefined)).toThrow('[serverless-dash0] Missing "custom.dash0"');
  });

  it('throws if endpoint is missing', () => {
    const { endpoint, ...rest } = VALID_CONFIG;
    expect(() => validateConfig(rest)).toThrow('"custom.dash0.endpoint" is required');
  });

  it('throws if endpoint is empty string', () => {
    expect(() => validateConfig({ ...VALID_CONFIG, endpoint: '' })).toThrow('"custom.dash0.endpoint" is required');
  });

  it('throws if layerVersion is missing', () => {
    const { layerVersion, ...rest } = VALID_CONFIG;
    expect(() => validateConfig(rest)).toThrow('"custom.dash0.layerVersion" is required');
  });

  it('throws if layerVersion is 0', () => {
    expect(() => validateConfig({ ...VALID_CONFIG, layerVersion: 0 })).toThrow('"custom.dash0.layerVersion"');
  });

  it('throws if layerVersion is negative', () => {
    expect(() => validateConfig({ ...VALID_CONFIG, layerVersion: -1 })).toThrow('"custom.dash0.layerVersion"');
  });

  it('throws if layerVersion is a float', () => {
    expect(() => validateConfig({ ...VALID_CONFIG, layerVersion: 1.5 })).toThrow('"custom.dash0.layerVersion"');
  });

  it('passes with layerVersion "latest"', () => {
    const config = { ...VALID_CONFIG, layerVersion: 'latest' };
    const result = validateConfig(config);
    expect(result.layerVersion).toBe('latest');
  });

  it('passes with layerAccountId as string', () => {
    const config = { ...VALID_CONFIG, layerAccountId: '999888777666' };
    const result = validateConfig(config);
    expect(result.layerAccountId).toBe('999888777666');
  });

  it('coerces numeric layerAccountId to string', () => {
    const config = { ...VALID_CONFIG, layerAccountId: 285732642181 };
    const result = validateConfig(config);
    expect(result.layerAccountId).toBe('285732642181');
  });

  it('throws if neither token nor tokenSecretArn is provided', () => {
    const { token, ...rest } = VALID_CONFIG;
    expect(() => validateConfig(rest)).toThrow('Either "custom.dash0.token" or "custom.dash0.tokenSecretArn"');
  });

  it('throws if extensionLogLevel is invalid', () => {
    expect(() =>
      validateConfig({ ...VALID_CONFIG, extensionLogLevel: 'verbose' })
    ).toThrow('"custom.dash0.extensionLogLevel" must be one of');
  });

  it('passes with valid extensionLogLevel values', () => {
    for (const level of ['trace', 'debug', 'info', 'warn', 'error']) {
      expect(() => validateConfig({ ...VALID_CONFIG, extensionLogLevel: level })).not.toThrow();
    }
  });
});

export interface Dash0Config {
  endpoint: string;
  layerVersion: number | 'latest';
  layerAccountId?: string;
  layerNamePrefix?: string;
  token?: string;
  tokenSecretArn?: string;
  tokenSecretKey?: string;
  dataset?: string;
  extensionLogLevel?: string;
  disableAutoInstrumentation?: boolean | string;
  sendOnInvocationEnd?: boolean | string;
  disableTelemetryLogCollection?: boolean | string;
  disableTelemetryMetrics?: boolean | string;
  createPayloadLogRecords?: boolean | string;
  requestTimeout?: number;
  xrayTracesEnabled?: boolean | string;
  maskRules?: string;
  maskEnvVars?: string;
}

const VALID_LOG_LEVELS = ['trace', 'debug', 'info', 'warn', 'error'];

export function validateConfig(config: unknown): Dash0Config {
  if (!config || typeof config !== 'object') {
    throw new Error('[serverless-dash0] Missing "custom.dash0" configuration block');
  }

  const c = config as Record<string, unknown>;

  if (!c.endpoint || typeof c.endpoint !== 'string') {
    throw new Error('[serverless-dash0] "custom.dash0.endpoint" is required and must be a string');
  }

  if (c.layerVersion === undefined || c.layerVersion === null) {
    throw new Error('[serverless-dash0] "custom.dash0.layerVersion" is required (positive integer or "latest")');
  }
  if (c.layerVersion !== 'latest') {
    const parsed = Number(c.layerVersion);
    if (!Number.isInteger(parsed) || parsed < 1) {
      throw new Error('[serverless-dash0] "custom.dash0.layerVersion" must be a positive integer or "latest"');
    }
    c.layerVersion = parsed;
  }

  if (c.layerAccountId !== undefined) {
    c.layerAccountId = String(c.layerAccountId);
  }

  if (!c.token && !c.tokenSecretArn) {
    throw new Error(
      '[serverless-dash0] Either "custom.dash0.token" or "custom.dash0.tokenSecretArn" must be provided'
    );
  }

  if (c.extensionLogLevel !== undefined) {
    if (!VALID_LOG_LEVELS.includes(c.extensionLogLevel as string)) {
      throw new Error(
        `[serverless-dash0] "custom.dash0.extensionLogLevel" must be one of: ${VALID_LOG_LEVELS.join(', ')}`
      );
    }
  }

  return c as unknown as Dash0Config;
}

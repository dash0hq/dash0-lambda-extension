import { Dash0Config } from './config-validator';

const OPTIONAL_MAPPINGS: Record<string, string> = {
  dataset: 'DASH0_DATASET',
  extensionLogLevel: 'DASH0_EXTENSION_LOG_LEVEL',
  disableAutoInstrumentation: 'DASH0_DISABLE_AUTO_INSTRUMENTATION',
  sendOnInvocationEnd: 'DASH0_SEND_ON_INVOCATION_END',
  disableTelemetryLogCollection: 'DASH0_DISABLE_TELEMETRY_LOG_COLLECTION',
  createPayloadLogRecords: 'DASH0_CREATE_PAYLOAD_LOG_RECORDS',
  requestTimeout: 'DASH0_REQUEST_TIMEOUT',
  xrayTracesEnabled: 'DASH0_XRAY_TRACES_ENABLED',
  maskRules: 'DASH0_MASK_RULES',
  maskEnvVars: 'DASH0_MASK_ENV_VARS',
};

export function buildEnvironment(config: Dash0Config): Record<string, string> {
  const env: Record<string, string> = {};

  env.AWS_LAMBDA_EXEC_WRAPPER = '/opt/wrapper';
  env.DASH0_ENDPOINT = config.endpoint;

  if (config.token) {
    env.DASH0_TOKEN = config.token;
  }
  if (config.tokenSecretArn) {
    env.DASH0_TOKEN_SECRET_ARN = config.tokenSecretArn;
  }
  if (config.tokenSecretKey) {
    env.DASH0_TOKEN_SECRET_KEY = config.tokenSecretKey;
  }

  for (const [configKey, envVar] of Object.entries(OPTIONAL_MAPPINGS)) {
    const value = (config as unknown as Record<string, unknown>)[configKey];
    if (value !== undefined && value !== null) {
      env[envVar] = String(value);
    }
  }

  return env;
}

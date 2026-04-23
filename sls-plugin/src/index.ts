import { validateConfig } from './config-validator';
import { resolveLayerName } from './runtime-mapping';
import { buildLayerArn } from './layer-arn-builder';
import { buildEnvironment } from './env-injector';
import * as bundledVersions from './versions.json';

interface ServerlessService {
  provider: {
    region?: string;
    runtime?: string;
  };
  custom?: {
    dash0?: unknown;
  };
  functions: Record<string, ServerlessFunction>;
}

interface ServerlessFunction {
  runtime?: string;
  handler?: string;
  image?: unknown;
  layers?: string[];
  environment?: Record<string, string>;
  [key: string]: unknown;
}

interface ServerlessInstance {
  cli: { log(msg: string): void };
  getProvider(name: string): unknown;
  service: ServerlessService;
}

interface ServerlessOptions {
  region?: string;
}

interface LogUtils {
  notice(msg: string): void;
  warning(msg: string): void;
  error(msg: string): void;
}

class ServerlessDash0Plugin {
  private serverless: ServerlessInstance;
  private options: ServerlessOptions;
  private log: (msg: string) => void;
  private warn: (msg: string) => void;
  public hooks: Record<string, () => void>;

  constructor(
    serverless: ServerlessInstance,
    options: ServerlessOptions,
    utils?: { log?: LogUtils }
  ) {
    this.serverless = serverless;
    this.options = options;

    this.log = utils?.log
      ? (msg: string) => utils.log!.notice(msg)
      : (msg: string) => serverless.cli.log(msg);

    this.warn = utils?.log
      ? (msg: string) => utils.log!.warning(msg)
      : (msg: string) => serverless.cli.log(`WARNING: ${msg}`);

    this.hooks = {
      'before:package:initialize': () => this.addDash0Tracing(),
    };
  }

  private getBundledVersion(layerName: string): number {
    const version = (bundledVersions as Record<string, number>)[layerName];
    if (!version) {
      throw new Error(
        `[serverless-dash0] No bundled version found for layer "${layerName}". Please specify an explicit layerVersion.`
      );
    }
    return version;
  }

  private addDash0Tracing(): void {
    const dash0Config = validateConfig(this.serverless.service.custom?.dash0);

    const region =
      this.options.region ||
      this.serverless.service.provider.region ||
      process.env.AWS_REGION ||
      process.env.AWS_DEFAULT_REGION ||
      'us-east-1';

    const dash0Env = buildEnvironment(dash0Config);
    const functions = this.serverless.service.functions;
    const useLatest = dash0Config.layerVersion === 'latest';

    for (const [funcName, funcDef] of Object.entries(functions)) {
      const traced = funcDef['dash0-traced'];
      if (traced !== true && traced !== 'true') {
        continue;
      }

      if (funcDef.image) {
        this.warn(
          `[serverless-dash0] Skipping function "${funcName}": container image functions cannot use Lambda layers. Use the Dash0 Docker image instead.`
        );
        continue;
      }

      const runtime = funcDef.runtime || this.serverless.service.provider.runtime;
      const layerName = resolveLayerName(runtime);

      if (!layerName) {
        this.warn(
          `[serverless-dash0] Skipping function "${funcName}": unsupported runtime "${runtime ?? 'undefined'}". Supported: nodejs*, python*, java*.`
        );
        continue;
      }

      const layerVersion = useLatest
        ? this.getBundledVersion(layerName)
        : dash0Config.layerVersion as number;

      const layerArn = buildLayerArn(region, layerName, layerVersion, dash0Config.layerAccountId, dash0Config.layerNamePrefix);

      // Append layer (no duplicates)
      funcDef.layers = funcDef.layers || [];
      if (!funcDef.layers.includes(layerArn)) {
        funcDef.layers.push(layerArn);
      }

      // Merge env vars (never overwrite existing)
      funcDef.environment = funcDef.environment || {};
      for (const [key, value] of Object.entries(dash0Env)) {
        if (funcDef.environment[key] === undefined) {
          funcDef.environment[key] = value;
        }
      }

      this.log(`[serverless-dash0] Configured tracing for function "${funcName}" (${runtime}, layer v${layerVersion})`);
    }
  }
}

module.exports = ServerlessDash0Plugin;

import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import { execSync } from 'child_process';
import * as path from 'path';

// Public Dash0 Node.js layer (see repo README / release page for versions).
const DASH0_LAYER_ACCOUNT = '115813213817';
const DEFAULT_DASH0_NODE_LAYER_VERSION = '5';

// Public AWS Lambda Web Adapter layer, https://github.com/awslabs/aws-lambda-web-adapter
const LWA_LAYER_ACCOUNT = '753240598075';
const DEFAULT_LWA_LAYER_VERSION = '28';

export class PlaygroundStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? 'sup1312-';

    const dash0Token = process.env.DASH0_TOKEN ?? process.env.DASH0_DEV_API_TOKEN;
    if (!dash0Token) {
      throw new Error('Set DASH0_TOKEN (or DASH0_DEV_API_TOKEN) before deploying');
    }
    const dash0Endpoint =
      process.env.DASH0_ENDPOINT ?? 'https://ingress.eu-west-1.aws.dash0-dev.com:4318';

    const dash0LayerArn =
      process.env.DASH0_NODE_LAYER_ARN ??
      `arn:aws:lambda:${this.region}:${DASH0_LAYER_ACCOUNT}:layer:dash0-extension-node:${DEFAULT_DASH0_NODE_LAYER_VERSION}`;
    const lwaLayerArn =
      process.env.LWA_LAYER_ARN ??
      `arn:aws:lambda:${this.region}:${LWA_LAYER_ACCOUNT}:layer:LambdaAdapterLayerX86:${DEFAULT_LWA_LAYER_VERSION}`;

    const dash0Layer = lambda.LayerVersion.fromLayerVersionArn(this, 'Dash0NodeLayer', dash0LayerArn);
    const lwaLayer = lambda.LayerVersion.fromLayerVersionArn(this, 'WebAdapterLayer', lwaLayerArn);

    // The proposed intermediate wrapper: occupies AWS_LAMBDA_EXEC_WRAPPER and
    // chains Dash0's /opt/wrapper into the Web Adapter's /opt/bootstrap.
    const chainedWrapperLayer = new lambda.LayerVersion(this, 'ChainedWrapperLayer', {
      layerVersionName: `${prefix}dash0-web-adapter-chained-wrapper`,
      code: lambda.Code.fromAsset(path.join(__dirname, '../chained-wrapper-layer')),
      compatibleRuntimes: [lambda.Runtime.NODEJS_20_X, lambda.Runtime.NODEJS_22_X],
      description:
        'Chained AWS_LAMBDA_EXEC_WRAPPER combining the Dash0 wrapper with the AWS Lambda Web Adapter (SUP-1312)',
    });

    // One shared code asset for all scenarios (express app + run.sh + plain
    // handler). Bundles locally when node/npm are available (all dependencies
    // are pure JS, so host-built node_modules are portable), otherwise falls
    // back to Docker bundling.
    const appDir = path.join(__dirname, '../app');
    const appCode = lambda.Code.fromAsset(appDir, {
      bundling: {
        image: lambda.Runtime.NODEJS_22_X.bundlingImage,
        command: [
          'bash',
          '-c',
          'npm ci --no-audit --no-fund --cache /tmp/.npm && cp -au . /asset-output',
        ],
        local: {
          tryBundle(outputDir: string): boolean {
            try {
              execSync('npm --version', { stdio: 'ignore' });
            } catch {
              return false;
            }
            execSync(`cp -a "${appDir}/." "${outputDir}/"`, { stdio: 'inherit' });
            execSync('npm ci --no-audit --no-fund', { cwd: outputDir, stdio: 'inherit' });
            return true;
          },
        },
      },
    });

    const role = new iam.Role(this, 'PlaygroundLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
      ],
    });

    const logGroup = new logs.LogGroup(this, 'PlaygroundLogGroup', {
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      retention: logs.RetentionDays.ONE_WEEK,
    });

    const dash0Env: Record<string, string> = {
      DASH0_TOKEN: dash0Token,
      DASH0_ENDPOINT: dash0Endpoint,
      DASH0_EXTENSION_LOG_LEVEL: process.env.DASH0_EXTENSION_LOG_LEVEL ?? 'info',
    };
    if (process.env.DASH0_DATASET) {
      dash0Env.DASH0_DATASET = process.env.DASH0_DATASET;
    }

    const scenarios: Array<{
      name: string;
      description: string;
      handler: string;
      layers: lambda.ILayerVersion[];
      environment: Record<string, string>;
    }> = [
      {
        // The customer's app as-is: Web Adapter serving the express app, no Dash0.
        name: '01-adapter-baseline',
        description: 'Web Adapter only - proves the web app works',
        handler: 'run.sh',
        layers: [lwaLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/bootstrap',
        },
      },
      {
        // Dash0 working as documented: plain handler, no Web Adapter.
        name: '02-dash0-baseline',
        description: 'Dash0 extension only - proves the extension works',
        handler: 'handler.handler',
        layers: [dash0Layer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
          ...dash0Env,
        },
      },
      {
        // The combination the ticket asks for: chained wrapper, Dash0
        // auto-instrumentation traces the express app.
        name: '03-chained',
        description: 'Chained wrapper: Dash0 auto-instrumentation + Web Adapter',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer, chainedWrapperLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/dash0-adapter-wrapper',
          ...dash0Env,
        },
      },
      {
        // Starling's current state: chained wrapper AND an in-app OTel SDK.
        // Expected to log "Attempted duplicate registration of ..." because the
        // Dash0 distro (loaded first via NODE_OPTIONS) already owns the globals.
        name: '04-chained-app-sdk',
        description: 'Repro: chained wrapper + in-app OTel SDK -> duplicate registration',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer, chainedWrapperLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/dash0-adapter-wrapper',
          INIT_APP_OTEL: 'true',
          OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:9009',
          ...dash0Env,
        },
      },
      {
        // Proposed resolution A: keep the chained wrapper for the Runtime API
        // proxy (invocation context, payloads, logs), but disable Dash0
        // auto-instrumentation so the app's own SDK is the only tracer. The app
        // SDK exports to the extension's local OTLP receiver.
        name: '05-app-sdk-via-proxy',
        description: 'Chained wrapper + auto-instr. disabled; in-app SDK -> extension OTLP (127.0.0.1:9009)',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer, chainedWrapperLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/dash0-adapter-wrapper',
          DASH0_DISABLE_AUTO_INSTRUMENTATION: 'true',
          INIT_APP_OTEL: 'true',
          OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:9009',
          ...dash0Env,
        },
      },
      {
        // Proposed resolution B (the "logs-only" model from the ticket): the
        // wrapper slot stays with the Web Adapter, the Dash0 layer is attached
        // without its wrapper (extension still collects logs via the Telemetry
        // API), and the in-app SDK exports to the extension's OTLP receiver.
        name: '06-no-dash0-wrapper',
        description: 'Web Adapter wrapper only; Dash0 layer without wrapper; in-app SDK -> extension OTLP',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/bootstrap',
          INIT_APP_OTEL: 'true',
          OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:9009',
          ...dash0Env,
        },
      },
      {
        // Proposed resolution C ("Model A" from the ticket): the in-app SDK
        // exports straight to the Dash0 ingress over the internet, bypassing
        // the extension for traces. The Dash0 layer stays attached for logs,
        // metrics, and telemetry-derived invocation spans.
        name: '07-app-sdk-direct',
        description: 'Web Adapter wrapper only; in-app SDK exports directly to the Dash0 ingress',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/bootstrap',
          INIT_APP_OTEL: 'true',
          APP_OTEL_EXPORT_MODE: 'direct',
          ...dash0Env,
        },
      },
      {
        // Scenario 03 + AWS_LWA_LAMBDA_RUNTIME_API_PROXY: the Web Adapter's
        // own knob for runtime-api-proxy extensions. Its extension process
        // polls the Runtime API through the Dash0 proxy, so the extension
        // regains the invocation context that scenario 03 was missing, while
        // the chained wrapper still injects the Dash0 auto-instrumentation.
        name: '08-lwa-proxy-chained',
        description: 'Chained wrapper + AWS_LWA_LAMBDA_RUNTIME_API_PROXY -> Dash0 auto-instrumentation full path',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer, chainedWrapperLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/dash0-adapter-wrapper',
          AWS_LWA_LAMBDA_RUNTIME_API_PROXY: '127.0.0.1:9009',
          ...dash0Env,
        },
      },
      {
        // Scenario 06 + AWS_LWA_LAMBDA_RUNTIME_API_PROXY: no wrapper chaining
        // at all. The app's own SDK exports to the extension's local OTLP
        // receiver, and the LWA polls through the Dash0 proxy so those spans
        // can be associated with invocations.
        name: '09-lwa-proxy-app-sdk',
        description: 'No chaining; AWS_LWA_LAMBDA_RUNTIME_API_PROXY + in-app SDK -> extension OTLP',
        handler: 'run.sh',
        layers: [dash0Layer, lwaLayer],
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/bootstrap',
          AWS_LWA_LAMBDA_RUNTIME_API_PROXY: '127.0.0.1:9009',
          INIT_APP_OTEL: 'true',
          OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:9009',
          ...dash0Env,
        },
      },
    ];

    for (const scenario of scenarios) {
      const fn = new lambda.Function(this, `Fn-${scenario.name}`, {
        functionName: `${prefix}${scenario.name}`,
        description: scenario.description,
        runtime: lambda.Runtime.NODEJS_22_X,
        architecture: lambda.Architecture.X86_64,
        handler: scenario.handler,
        code: appCode,
        layers: scenario.layers,
        role,
        memorySize: 1024,
        timeout: cdk.Duration.seconds(30),
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
        environment: {
          SCENARIO: scenario.name,
          OTEL_SERVICE_NAME: `${prefix}${scenario.name}`,
          PORT: '8080',
          ...scenario.environment,
        },
      });

      const url = fn.addFunctionUrl({
        authType: lambda.FunctionUrlAuthType.NONE,
      });

      new cdk.CfnOutput(this, `Url-${scenario.name}`, {
        key: `url${scenario.name.replace(/[^a-zA-Z0-9]/g, '')}`,
        value: url.url,
        description: scenario.description,
      });
    }

    new cdk.CfnOutput(this, 'LogGroupName', { value: logGroup.logGroupName });
    new cdk.CfnOutput(this, 'Dash0LayerArnUsed', { value: dash0LayerArn });
    new cdk.CfnOutput(this, 'WebAdapterLayerArnUsed', { value: lwaLayerArn });
  }
}

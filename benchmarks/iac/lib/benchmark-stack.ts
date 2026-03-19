import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as cr from 'aws-cdk-lib/custom-resources';
import * as path from 'path';

function getLatestLayerVersion(scope: Construct, id: string, layerName: string): lambda.ILayerVersion {
  const stack = cdk.Stack.of(scope);

  const latestVersion = new cr.AwsCustomResource(scope, `${id}LatestVersion`, {
    onUpdate: {
      service: 'Lambda',
      action: 'listLayerVersions',
      parameters: {
        LayerName: `arn:aws:lambda:${stack.region}:${stack.account}:layer:${layerName}`,
        MaxItems: 1,
      },
      physicalResourceId: cr.PhysicalResourceId.of(Date.now().toString()),
    },
    policy: cr.AwsCustomResourcePolicy.fromSdkCalls({
      resources: cr.AwsCustomResourcePolicy.ANY_RESOURCE,
    }),
  });

  const layerArn = `arn:aws:lambda:${stack.region}:${stack.account}:layer:${layerName}:${latestVersion.getResponseField('LayerVersions.0.Version')}`;

  return lambda.LayerVersion.fromLayerVersionArn(scope, id, layerArn);
}

interface RuntimeConfig {
  name: string;
  runtimes: lambda.Runtime[];
  layerName: string;
  handler: string;
  code: lambda.Code;
  memorySize: number;
}

// OSS OpenTelemetry Lambda layer versions (from account 184161586896)
// https://github.com/open-telemetry/opentelemetry-lambda/releases
const OSS_OTEL_LAYERS: Record<string, { layerName: string; version: number }> = {
  python: { layerName: 'opentelemetry-python-0_18_0', version: 2 },
  node: { layerName: 'opentelemetry-nodejs-0_20_0', version: 1 },
  java: { layerName: 'opentelemetry-javaagent-0_18_0', version: 1 },
};
const OSS_OTEL_COLLECTOR = { layerName: 'opentelemetry-collector-amd64-0_20_0', version: 1 };
const OSS_OTEL_ACCOUNT = '184161586896';

export class BenchmarkStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';

    const role = new iam.Role(this, 'BenchmarkLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
      ],
    });

    const logGroup = new logs.LogGroup(this, 'BenchmarkLogGroup', {
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      retention: logs.RetentionDays.ONE_WEEK,
    });

    const pythonCode = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/python'));

    const nodeCode = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/node'));

    const javaCode = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/java'), {
      bundling: {
        image: lambda.Runtime.JAVA_17.bundlingImage,
        command: [
          'bash', '-c',
          'chmod +x gradlew && ./gradlew buildZip && cd /asset-output && jar xf /asset-input/build/distributions/benchmark-java-1.0-SNAPSHOT.zip',
        ],
      },
    });

    const runtimeConfigs: RuntimeConfig[] = [
      {
        name: 'python',
        runtimes: [
          lambda.Runtime.PYTHON_3_10,
          lambda.Runtime.PYTHON_3_11,
          lambda.Runtime.PYTHON_3_12,
          lambda.Runtime.PYTHON_3_13,
          lambda.Runtime.PYTHON_3_14,
        ],
        layerName: `${prefix}dash0-extension-python`,
        handler: 'handler.handler',
        code: pythonCode,
        memorySize: 128,
      },
      {
        name: 'node',
        runtimes: [
          lambda.Runtime.NODEJS_20_X,
          lambda.Runtime.NODEJS_22_X,
          lambda.Runtime.NODEJS_24_X,
        ],
        layerName: `${prefix}dash0-extension-node`,
        handler: 'handler.handler',
        code: nodeCode,
        memorySize: 128,
      },
      {
        name: 'java',
        runtimes: [
          lambda.Runtime.JAVA_17,
          lambda.Runtime.JAVA_21,
          lambda.Runtime.JAVA_25,
        ],
        layerName: `${prefix}dash0-extension-java`,
        handler: 'org.example.BenchmarkHandler::handleRequest',
        code: javaCode,
        memorySize: 512,
      },
    ];

    // Extension-only: manual layer with no auto-instrumentation (Node.js 24)
    const manualLayer = getLatestLayerVersion(this, 'manualLayer', `${prefix}dash0-extension-manual`);
    new lambda.Function(this, 'extension-only-nodejs24-x', {
      functionName: `${prefix}bench-extension-only-nodejs24-x`,
      runtime: lambda.Runtime.NODEJS_24_X,
      memorySize: 128,
      handler: 'handler.handler',
      architecture: lambda.Architecture.X86_64,
      timeout: cdk.Duration.seconds(30),
      code: nodeCode,
      layers: [manualLayer],
      role,
      environment: {
        AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
        DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN ?? 'benchmark-dummy-token',
        DASH0_ENDPOINT: process.env.DASH0_ENDPOINT ?? 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
        DASH0_EXTENSION_LOG_LEVEL: 'warn',
      },
      logGroup,
      loggingFormat: lambda.LoggingFormat.TEXT,
    });

    // OSS OTel collector layer (shared across all runtimes)
    const ossCollectorArn = `arn:aws:lambda:${this.region}:${OSS_OTEL_ACCOUNT}:layer:${OSS_OTEL_COLLECTOR.layerName}:${OSS_OTEL_COLLECTOR.version}`;
    const ossCollectorLayer = lambda.LayerVersion.fromLayerVersionArn(this, 'oss-otel-collector', ossCollectorArn);

    for (const config of runtimeConfigs) {
      const layer = getLatestLayerVersion(this, `${config.name}Layer`, config.layerName);

      // OSS OTel language layer for this runtime
      const ossConfig = OSS_OTEL_LAYERS[config.name];
      const ossLayerArn = `arn:aws:lambda:${this.region}:${OSS_OTEL_ACCOUNT}:layer:${ossConfig.layerName}:${ossConfig.version}`;
      const ossLayer = lambda.LayerVersion.fromLayerVersionArn(this, `oss-otel-${config.name}`, ossLayerArn);

      for (const runtime of config.runtimes) {
        const runtimeName = runtime.name.replace(/\./g, '-');

        // Baseline: no layer, no wrapper
        new lambda.Function(this, `baseline-${runtimeName}`, {
          functionName: `${prefix}bench-baseline-${runtimeName}`,
          runtime,
          memorySize: config.memorySize,
          handler: config.handler,
          architecture: lambda.Architecture.X86_64,
          timeout: cdk.Duration.seconds(30),
          code: config.code,
          role,
          logGroup,
          loggingFormat: lambda.LoggingFormat.TEXT,
        });

        // Instrumented: with Dash0 layer and wrapper
        new lambda.Function(this, `instrumented-${runtimeName}`, {
          functionName: `${prefix}bench-instrumented-${runtimeName}`,
          runtime,
          memorySize: config.memorySize,
          handler: config.handler,
          architecture: lambda.Architecture.X86_64,
          timeout: cdk.Duration.seconds(30),
          code: config.code,
          layers: [layer],
          role,
          environment: {
            AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
            DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN ?? 'benchmark-dummy-token',
            DASH0_ENDPOINT: process.env.DASH0_ENDPOINT ?? 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
            DASH0_EXTENSION_LOG_LEVEL: 'warn',
          },
          logGroup,
          loggingFormat: lambda.LoggingFormat.TEXT,
        });

        // OSS OTel: open-source OpenTelemetry language layer + collector layer
        new lambda.Function(this, `oss-otel-${runtimeName}`, {
          functionName: `${prefix}bench-oss-otel-${runtimeName}`,
          runtime,
          memorySize: config.memorySize,
          handler: config.handler,
          architecture: lambda.Architecture.X86_64,
          timeout: cdk.Duration.seconds(30),
          code: config.code,
          layers: [ossLayer, ossCollectorLayer],
          role,
          environment: {
            AWS_LAMBDA_EXEC_WRAPPER: '/opt/otel-handler',
            OTEL_SERVICE_NAME: `bench-oss-otel-${runtimeName}`,
          },
          logGroup,
          loggingFormat: lambda.LoggingFormat.TEXT,
        });
      }
    }
  }
}

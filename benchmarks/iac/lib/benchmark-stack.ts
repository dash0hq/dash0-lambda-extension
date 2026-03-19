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

    for (const config of runtimeConfigs) {
      const layer = getLatestLayerVersion(this, `${config.name}Layer`, config.layerName);

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

        // Instrumented: with layer and wrapper
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
      }
    }
  }
}

import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as path from 'path';

// Distro masking vs extension masking (DASH0_MASK_IN_EXTENSION) on real Lambda.
// One layer per runtime; the variants differ only in environment variables:
//   baseline: no layer
//   distro:   layer, default masking (the distro masks, the extension does not)
//   ext:      DASH0_MASK_IN_EXTENSION=true, distro masking switched off
//   none:     distro masking off, extension masking off
// Layer ARNs come from MASKING_LAYER_NODE / _PYTHON / _JAVA.

const MEMORY_SIZES = (process.env.MASKING_MEMORY_SIZES ?? '128,512,1024').split(',').map(Number);
const VARIANTS = ['baseline', 'distro', 'ext', 'none'] as const;

function requiredEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

export class MaskingBenchmarkStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const lambdas = path.join(__dirname, '../lambdas');

    // Echo target for the workload's HTTP calls, and the OTLP endpoint for the extension.
    const sink = new lambda.Function(this, 'sink', {
      functionName: `${prefix}sink`,
      runtime: lambda.Runtime.NODEJS_22_X,
      architecture: lambda.Architecture.ARM_64,
      memorySize: 1024,
      handler: 'handler.handler',
      code: lambda.Code.fromAsset(path.join(lambdas, 'masking/sink')),
      timeout: cdk.Duration.seconds(10),
      logGroup: new logs.LogGroup(this, 'sink-logs', {
        logGroupName: `/aws/lambda/${prefix}sink`,
        removalPolicy: cdk.RemovalPolicy.DESTROY,
        retention: logs.RetentionDays.ONE_DAY,
      }),
    });
    const sinkUrl = sink.addFunctionUrl({ authType: lambda.FunctionUrlAuthType.NONE });
    const sinkHost = cdk.Fn.select(2, cdk.Fn.split('/', sinkUrl.url));

    const runtimes = [
      {
        name: 'node22',
        runtime: lambda.Runtime.NODEJS_22_X,
        handler: 'handler.handler',
        code: lambda.Code.fromAsset(path.join(lambdas, 'masking/node')),
        layerArn: requiredEnv('MASKING_LAYER_NODE'),
        distroOff: { LUMIGO_SECRET_MASKING_REGEX: '[]' } as Record<string, string>,
      },
      {
        name: 'python313',
        runtime: lambda.Runtime.PYTHON_3_13,
        handler: 'handler.handler',
        code: lambda.Code.fromAsset(path.join(lambdas, 'masking/python'), {
          bundling: {
            image: lambda.Runtime.PYTHON_3_13.bundlingImage,
            command: ['bash', '-c', 'pip install -r requirements.txt -t /asset-output && cp handler.py /asset-output'],
          },
        }),
        layerArn: requiredEnv('MASKING_LAYER_PYTHON'),
        distroOff: { LUMIGO_SECRET_MASKING_REGEX: '[]' } as Record<string, string>,
      },
      {
        name: 'java21',
        runtime: lambda.Runtime.JAVA_21,
        handler: 'org.example.MaskingWorkloadHandler::handleRequest',
        code: lambda.Code.fromAsset(path.join(lambdas, 'java'), {
          bundling: {
            image: lambda.Runtime.JAVA_21.bundlingImage,
            environment: { GRADLE_USER_HOME: '/tmp/.gradle' },
            command: [
              'bash', '-c',
              'chmod +x gradlew && ./gradlew buildZip && cd /asset-output && jar xf /asset-input/build/distributions/benchmark-java-1.0-SNAPSHOT.zip',
            ],
          },
        }),
        layerArn: requiredEnv('MASKING_LAYER_JAVA'),
        // The wrapper appends -javaagent to JAVA_TOOL_OPTIONS; the system property beats DASH0_MASK_RULES.
        distroOff: { JAVA_TOOL_OPTIONS: '-Ddash0.mask.rules=[]' } as Record<string, string>,
      },
    ];

    for (const rt of runtimes) {
      const layer = lambda.LayerVersion.fromLayerVersionArn(this, `${rt.name}-layer`, rt.layerArn);
      for (const memorySize of MEMORY_SIZES) {
        for (const variant of VARIANTS) {
          const functionName = `${prefix}${rt.name}-${variant}-${memorySize}`;
          const dash0 = {
            AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
            DASH0_TOKEN: 'benchmark-dummy-token',
            DASH0_ENDPOINT: `https://${sinkHost}:443`,
            DASH0_EXTENSION_LOG_LEVEL: 'warn',
          };
          const environment: Record<string, string> = {
            ECHO_URL: `https://${sinkHost}/echo`,
            ...(variant === 'baseline' ? {} : dash0),
            ...(variant === 'ext' ? { DASH0_MASK_IN_EXTENSION: 'true' } : {}),
            ...(variant === 'ext' || variant === 'none' ? rt.distroOff : {}),
          };
          new lambda.Function(this, functionName, {
            functionName,
            runtime: rt.runtime,
            memorySize,
            handler: rt.handler,
            architecture: lambda.Architecture.ARM_64,
            timeout: cdk.Duration.seconds(60),
            code: rt.code,
            layers: variant === 'baseline' ? [] : [layer],
            environment,
            // JSON logs with DEBUG system logs carry platform.runtimeDone next to platform.report.
            loggingFormat: lambda.LoggingFormat.JSON,
            systemLogLevelV2: lambda.SystemLogLevel.DEBUG,
            applicationLogLevelV2: lambda.ApplicationLogLevel.INFO,
            logGroup: new logs.LogGroup(this, `${functionName}-logs`, {
              logGroupName: `/aws/lambda/${functionName}`,
              removalPolicy: cdk.RemovalPolicy.DESTROY,
              retention: logs.RetentionDays.ONE_DAY,
            }),
          });
        }
      }
    }

    new cdk.CfnOutput(this, 'SinkUrl', { value: sinkUrl.url });
  }
}

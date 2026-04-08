import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as ecr_assets from 'aws-cdk-lib/aws-ecr-assets';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as lambdaNodejs from 'aws-cdk-lib/aws-lambda-nodejs';
import * as path from 'path';
import { createPythonCode } from './python-tracing-scenarios-stack';
import { getLatestLayerVersion, importSharedResources } from './shared-resources-stack';

function createLambdas(
    scope: Construct,
    runtimes: lambda.Runtime[],
    layer: lambda.ILayerVersion,
    role: iam.IRole,
    logGroup: logs.ILogGroup,
    prefix: string,
    overrides?: { handler?: string; code?: lambda.Code; memorySize?: number; dash0TokenSecretArn?: string }
) {
  const latestRuntime = runtimes[runtimes.length - 1];
  for (const runtime of runtimes) {
    for (const architecture of [lambda.Architecture.X86_64, lambda.Architecture.ARM_64]) {
      for (const invocationEnd of ["true", "false"]) {
        for (const traced of ["true", "false"]) {
          for (const scenario of ["success", "timeout", "outofmemory", "importerror", "exception"]) {
            if (runtime.family === lambda.RuntimeFamily.NODEJS && scenario === "outofmemory") {
              // seems to be impossible to make nodejs lambda run out of memory in a reliable way
              // essentially it ends up throwing a timeout instead
              continue;
            }
            if (runtime.family === lambda.RuntimeFamily.JAVA && (scenario === "importerror" || scenario === "outofmemory")) {
              continue;
            }
            const runtimeName = runtime.name.replace(/\./g, '-');
            const useSecretManager = overrides?.dash0TokenSecretArn
                && runtime === latestRuntime
                && scenario === "success";
            const environment: any = {
              AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
              DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
              DASH0_EXTENSION_LOG_LEVEL: "info",
              DASH0_SEND_ON_INVOCATION_END: invocationEnd,
            };
            if (useSecretManager) {
              environment["DASH0_TOKEN_SECRET_ARN"] = overrides!.dash0TokenSecretArn!;
            } else {
              environment["DASH0_TOKEN"] = process.env.DASH0_DEV_API_TOKEN!;
            }
            if (traced === "false") {
              environment["DASH0_DISABLE_AUTO_INSTRUMENTATION"] = "true";
            }
            if (scenario === "timeout") {
              environment["SLEEP_DURATION_MS"] = "20000";
            }
            if (runtime.family === lambda.RuntimeFamily.NODEJS) {
              environment["DASH0_MASK_RULES"] = '[".*masked_field.*"]';
              environment["MASKED_FIELD"] = 'sensitive information';
            }

            const functionName = `${prefix}${runtimeName}-${scenario}-${traced}-invocation-end-${invocationEnd}-${architecture.name}`;
            const handler = overrides?.handler ?? `${scenario}.handler`;
            const code = overrides?.code ?? lambda.Code.fromAsset(path.join(__dirname, '../lambdas/node'));
            const memorySize = overrides?.memorySize ?? 128;

            new lambda.Function(scope, functionName, {
              functionName: functionName,
              runtime: runtime,
              memorySize,
              handler,
              architecture: architecture,
              timeout: cdk.Duration.seconds(10),
              code,
              layers: [layer],
              role: role,
              environment,
              logGroup: logGroup,
              loggingFormat: lambda.LoggingFormat.TEXT,
            });
          }
        }
      }
    }
  }
}

export class PythonStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const { role, logGroup, secretArn } = importSharedResources(this);
    const layer = getLatestLayerVersion(this, 'pythonLayer', `${prefix}dash0-extension-python`);

    const runtimes = [
      lambda.Runtime.PYTHON_3_10,
      lambda.Runtime.PYTHON_3_11,
      lambda.Runtime.PYTHON_3_12,
      lambda.Runtime.PYTHON_3_13,
      lambda.Runtime.PYTHON_3_14,
    ];

    createLambdas(this, runtimes, layer, role, logGroup, prefix, {
      code: createPythonCode(),
      dash0TokenSecretArn: secretArn,
    });

    // Dependency conflict test lambdas
    const dependencyConflictCode = lambda.Code.fromAsset(
      path.join(__dirname, '../lambdas/python/dependency-conflict'),
      {
        bundling: {
          image: lambda.Runtime.PYTHON_3_12.bundlingImage,
          command: [
            'bash', '-c',
            'pip install -r requirements.txt -t /asset-output && cp -au . /asset-output',
          ],
        },
      },
    );

    const dependencyConflictRuntimes = runtimes.filter(r => r !== lambda.Runtime.PYTHON_3_14);
    for (const runtime of dependencyConflictRuntimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      new lambda.Function(this, `dependency-conflict-${runtimeName}`, {
        functionName: `${prefix}dependency-conflict-${runtimeName}`,
        runtime,
        memorySize: 128,
        handler: 'handler.handler',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        code: dependencyConflictCode,
        layers: [layer],
        role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
          DASH0_EXTENSION_LOG_LEVEL: 'info',
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

export class NodeStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const { role, logGroup, secretArn } = importSharedResources(this);
    const layer = getLatestLayerVersion(this, 'nodeLayer', `${prefix}dash0-extension-node`);

    const runtimes = [
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
    ];

    createLambdas(this, runtimes, layer, role, logGroup, prefix, {
      dash0TokenSecretArn: secretArn,
    });

    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      new lambda.Function(this, `single-traced-${runtimeName}`, {
        functionName: `${prefix}single-traced-${runtimeName}`,
        runtime,
        memorySize: 128,
        handler: 'success.handler',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas/node')),
        layers: [layer],
        role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
          DASH0_EXTENSION_LOG_LEVEL: "info",
          DASH0_XRAY_TRACES_ENABLED: "true",
        },
        tracing: lambda.Tracing.ACTIVE,
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }

    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      new lambdaNodejs.NodejsFunction(this, `cjs-success-${runtimeName}`, {
        functionName: `${prefix}cjs-success-${runtimeName}`,
        runtime,
        memorySize: 128,
        entry: path.join(__dirname, '../lambdas/node/check-cjs-bundle.ts'),
        handler: 'handler',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        bundling: {
          format: lambdaNodejs.OutputFormat.CJS,
        },
        layers: [layer],
        role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
          DASH0_EXTENSION_LOG_LEVEL: "info",
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

export class JavaStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const { role, logGroup, secretArn } = importSharedResources(this);
    const layer = getLatestLayerVersion(this, 'javaLayer', `${prefix}dash0-extension-java`);

    const javaCode = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/java'), {
      bundling: {
        image: lambda.Runtime.JAVA_17.bundlingImage,
        command: [
          'bash', '-c',
          'chmod +x gradlew && ./gradlew buildZip && cd /asset-output && jar xf /asset-input/build/distributions/lambda-java-1.0-SNAPSHOT.zip',
        ],
      },
    });

    const overrides = {
      handler: 'org.example.HelloHandler::handleRequest',
      code: javaCode,
      memorySize: 512,
      dash0TokenSecretArn: secretArn,
    };
    const runtimes = [lambda.Runtime.JAVA_25, lambda.Runtime.JAVA_21, lambda.Runtime.JAVA_17];

    createLambdas(this, runtimes, layer, role, logGroup, prefix, overrides);
  }
}

export class ManualStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const { role, logGroup } = importSharedResources(this);
    const layer = getLatestLayerVersion(this, 'manualLayer', `${prefix}dash0-extension-manual`);

    const runtimes = [
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
    ];
    const code = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/manual'), {
      bundling: {
        image: lambda.Runtime.NODEJS_24_X.bundlingImage,
        command: [
          'bash', '-c',
          'npm ci --cache /tmp/.npm && cp -au . /asset-output'
        ],
      },
    });
    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      new lambda.Function(this, `manual-instrumentation-${runtimeName}`, {
        functionName: `${prefix}manual-instrumentation-${runtimeName}`,
        runtime,
        memorySize: 512,
        handler: 'index.hello',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        code,
        layers: [layer],
        role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
          DASH0_EXTENSION_LOG_LEVEL: "info",
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

export class DockerizedStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const { role, logGroup } = importSharedResources(this);

    const account = process.env.CDK_DEFAULT_ACCOUNT;
    const region = process.env.CDK_DEFAULT_REGION;

    for (const runtime of ["python", "node", "java"]) {
      for (const architecture of [lambda.Architecture.X86_64, lambda.Architecture.ARM_64]) {
        const extensionImage = `${account}.dkr.ecr.${region}.amazonaws.com/${prefix}dash0-extension-${runtime}:latest`;
        const platform = architecture === lambda.Architecture.ARM_64
          ? ecr_assets.Platform.LINUX_ARM64
          : ecr_assets.Platform.LINUX_AMD64;
        new lambda.DockerImageFunction(this, `dockerized-${runtime}-${architecture.name}`, {
          functionName: `${prefix}dockerized-${runtime}-${architecture.name}`,
          code: lambda.DockerImageCode.fromImageAsset(path.join(__dirname, `../lambdas/dockerized-${runtime}`), {
            buildArgs: {
              EXTENSION_IMAGE: extensionImage,
            },
            extraHash: Date.now().toString(),
            platform,
          }),
          memorySize: 512,
          architecture,
          timeout: cdk.Duration.seconds(10),
          role,
          environment: {
            DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
            DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
            DASH0_EXTENSION_LOG_LEVEL: "info",
          },
          logGroup,
          loggingFormat: lambda.LoggingFormat.TEXT,
        });
      }
    }
  }
}

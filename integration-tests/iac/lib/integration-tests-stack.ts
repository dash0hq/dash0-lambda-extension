import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as ecr_assets from 'aws-cdk-lib/aws-ecr-assets';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as cr from 'aws-cdk-lib/custom-resources';
import * as path from 'path';
import { PythonTracingScenariosStack, createPythonCode } from './python-tracing-scenarios-stack';
import { NodeTracingScenariosStack } from './node-tracing-scenarios-stack';

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

interface SubStackProps extends cdk.NestedStackProps {
  role: iam.Role;
  layer: lambda.ILayerVersion;
  logGroup: logs.ILogGroup;
}

function createLambdas(
    scope: Construct,
    runtimes: lambda.Runtime[],
    layer: lambda.ILayerVersion,
    role: iam.Role,
    logGroup: logs.ILogGroup,
    overrides?: { handler?: string; code?: lambda.Code; memorySize?: number }
) {
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
            const environment: any = {
              AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
              DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
              DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
              DASH0_EXTENSION_LOG_LEVEL: "info",
              DASH0_SEND_ON_INVOCATION_END: invocationEnd,
              DASH0_MASK_RULES: '[".*masked_field.*"]',
              MASKED_FIELD: "sensitive information",
            };
            if (traced === "false") {
              environment["DASH0_DISABLE_AUTO_INSTRUMENTATION"] = "true";
            }
            if (scenario === "timeout") {
              environment["SLEEP_DURATION_MS"] = "20000";
            }

            const functionName = `${runtimeName}-${scenario}-${traced}-invocation-end-${invocationEnd}-${architecture.name}`;
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

class PythonStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: SubStackProps) {
    super(scope, id, props);

    const runtimes = [
      lambda.Runtime.PYTHON_3_10,
      lambda.Runtime.PYTHON_3_11,
      lambda.Runtime.PYTHON_3_12,
      lambda.Runtime.PYTHON_3_13,
      lambda.Runtime.PYTHON_3_14,
    ];

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup, {
      code: createPythonCode(),
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
        functionName: `dependency-conflict-${runtimeName}`,
        runtime,
        memorySize: 128,
        handler: 'handler.handler',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        code: dependencyConflictCode,
        layers: [props.layer],
        role: props.role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
          DASH0_EXTENSION_LOG_LEVEL: 'info',
        },
        logGroup: props.logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

class NodeStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: SubStackProps) {
    super(scope, id, props);

    const runtimes = [
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
    ];

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup);
  }
}

class JavaStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: SubStackProps) {
    super(scope, id, props);

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
    };

    createLambdas(this, [lambda.Runtime.JAVA_21], props.layer, props.role, props.logGroup, overrides);
    createLambdas(this, [lambda.Runtime.JAVA_17], props.layer, props.role, props.logGroup, overrides);
  }
}

class ManualStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: SubStackProps) {
    super(scope, id, props);

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
        functionName: `manual-instrumentation-${runtimeName}`,
        runtime,
        memorySize: 512,
        handler: 'index.hello',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        code,
        layers: [props.layer],
        role: props.role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
          DASH0_EXTENSION_LOG_LEVEL: "info",
        },
        logGroup: props.logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

interface DockerizedStackProps extends cdk.NestedStackProps {
  role: iam.Role;
  logGroup: logs.ILogGroup;
}

class DockerizedStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: DockerizedStackProps) {
    super(scope, id, props);

    const account = process.env.CDK_DEFAULT_ACCOUNT;
    const region = process.env.CDK_DEFAULT_REGION;

    for (const runtime of ["python", "node", "java"]) {
      for (const architecture of [lambda.Architecture.X86_64, lambda.Architecture.ARM_64]) {
        const extensionImage = `${account}.dkr.ecr.${region}.amazonaws.com/dash0-extension-${runtime}:latest`;
        const platform = architecture === lambda.Architecture.ARM_64
          ? ecr_assets.Platform.LINUX_ARM64
          : ecr_assets.Platform.LINUX_AMD64;
        new lambda.DockerImageFunction(this, `dockerized-${runtime}-${architecture.name}`, {
          functionName: `dockerized-${runtime}-${architecture.name}`,
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
          role: props.role,
          environment: {
            DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
            DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
            DASH0_EXTENSION_LOG_LEVEL: "info",
          },
          logGroup: props.logGroup,
          loggingFormat: lambda.LoggingFormat.TEXT,
        });
      }
    }
  }
}

export class IntegrationTestsStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const pythonLayer = getLatestLayerVersion(this, 'pythonLrapLayer', 'dash0-extension-python');
    const nodeLayer = getLatestLayerVersion(this, 'nodeLrapLayer', 'dash0-extension-node');
    const javaLayer = getLatestLayerVersion(this, 'javaLrapLayer', 'dash0-extension-java');
    const manualLayer = getLatestLayerVersion(this, 'manualLrapLayer', 'dash0-extension-manual');
    const role = new iam.Role(this, 'IntegrationTestsLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
      ],
    });

    const sharedLogGroup = new logs.LogGroup(this, 'IntegrationTestsLogGroup', {
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      retention: logs.RetentionDays.ONE_DAY,
    });

    new PythonStack(this, 'PythonStack', {
      role,
      layer: pythonLayer,
      logGroup: sharedLogGroup,
    });

    new NodeStack(this, 'NodeStack', {
      role,
      layer: nodeLayer,
      logGroup: sharedLogGroup,
    });

    new JavaStack(this, 'JavaStack', {
      role,
      layer: javaLayer,
      logGroup: sharedLogGroup,
    });

    new ManualStack(this, 'ManualStack', {
      role,
      layer: manualLayer,
      logGroup: sharedLogGroup,
    });

    new DockerizedStack(this, 'DockerizedStack', {
      role,
      logGroup: sharedLogGroup,
    });

    new PythonTracingScenariosStack(this, 'TracingScenariosStack', {
      layer: pythonLayer,
      logGroup: sharedLogGroup,
    });

    new NodeTracingScenariosStack(this, 'NodeTracingScenariosStack', {
      layer: nodeLayer,
      logGroup: sharedLogGroup,
    });
  }
}

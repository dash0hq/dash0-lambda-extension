import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as ecr_assets from 'aws-cdk-lib/aws-ecr-assets';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import * as cr from 'aws-cdk-lib/custom-resources';
import * as lambdaNodejs from 'aws-cdk-lib/aws-lambda-nodejs';
import * as path from 'path';
import { PythonTracingScenariosStack, createPythonCode } from './python-tracing-scenarios-stack';
import { NodeTracingScenariosStack } from './node-tracing-scenarios-stack';
import { JavaTracingScenariosStack } from './java-tracing-scenarios-stack';
import { DbTestingStack } from './db-testing-stack';
import { PYTHON_CDK_RUNTIMES, NODE_CDK_RUNTIMES, JAVA_CDK_RUNTIMES } from './runtime-utils';

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
  prefix: string;
  dash0TokenSecretArn?: string;
}

function createLambdas(
    scope: Construct,
    runtimes: lambda.Runtime[],
    layer: lambda.ILayerVersion,
    role: iam.Role,
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

            // Python 3.14 + the dash0 distribution's import graph is slow enough that
            // Lambda's recovery init (phase=invoke) can't run to completion within the
            // default 10s, so failure-mode tests see `timeout` instead of the expected
            // error type. Give those scenarios more headroom on 3.14 only.
            const timeoutSeconds =
              runtime === lambda.Runtime.PYTHON_3_14 &&
              (scenario === "importerror" || scenario === "outofmemory")
                ? 30
                : 10;

            new lambda.Function(scope, functionName, {
              functionName: functionName,
              runtime: runtime,
              memorySize,
              handler,
              architecture: architecture,
              timeout: cdk.Duration.seconds(timeoutSeconds),
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

    const runtimes = PYTHON_CDK_RUNTIMES;

    const pythonCode = createPythonCode();

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup, props.prefix, {
      code: pythonCode,
      dash0TokenSecretArn: props.dash0TokenSecretArn,
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
        functionName: `${props.prefix}dependency-conflict-${runtimeName}`,
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

    // API Gateway lambdas that always return 500
    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      for (const traced of [true, false]) {
        const suffix = traced ? runtimeName : `untraced-${runtimeName}`;
        const environment: Record<string, string> = {
          AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
          DASH0_EXTENSION_LOG_LEVEL: 'info',
        };
        if (!traced) {
          environment['DASH0_DISABLE_AUTO_INSTRUMENTATION'] = 'true';
        }

        const fn = new lambda.Function(this, `apigw-error-500-${suffix}`, {
          functionName: `${props.prefix}apigw-error-500-${suffix}`,
          runtime,
          memorySize: 128,
          handler: 'error_500.handler',
          architecture: lambda.Architecture.X86_64,
          timeout: cdk.Duration.seconds(10),
          code: pythonCode,
          layers: [props.layer],
          role: props.role,
          environment,
          logGroup: props.logGroup,
          loggingFormat: lambda.LoggingFormat.TEXT,
        });

        new apigateway.LambdaRestApi(this, `ApiGw500-${suffix}`, {
          restApiName: `${props.prefix}apigw-error-500-${suffix}`,
          handler: fn,
          proxy: true,
        });
      }
    }
  }
}

class NodeStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: SubStackProps) {
    super(scope, id, props);

    const runtimes = NODE_CDK_RUNTIMES;

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup, props.prefix, {
      dash0TokenSecretArn: props.dash0TokenSecretArn,
    });

    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      new lambda.Function(this, `single-traced-${runtimeName}`, {
        functionName: `${props.prefix}single-traced-${runtimeName}`,
        runtime,
        memorySize: 128,
        handler: 'success.handler',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas/node')),
        layers: [props.layer],
        role: props.role,
        environment: {
          AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
          DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
          DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
          DASH0_EXTENSION_LOG_LEVEL: "info",
          DASH0_XRAY_TRACES_ENABLED: "true",
        },
        tracing: lambda.Tracing.ACTIVE,
        logGroup: props.logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }

    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      new lambdaNodejs.NodejsFunction(this, `cjs-success-${runtimeName}`, {
        functionName: `${props.prefix}cjs-success-${runtimeName}`,
        runtime,
        memorySize: 128,
        entry: path.join(__dirname, '../lambdas/node/check-cjs-bundle.ts'),
        handler: 'handler',
        architecture: lambda.Architecture.X86_64,
        timeout: cdk.Duration.seconds(10),
        bundling: {
          format: lambdaNodejs.OutputFormat.CJS,
        },
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
      dash0TokenSecretArn: props.dash0TokenSecretArn,
    };
    const runtimes = JAVA_CDK_RUNTIMES;

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup, props.prefix, overrides);
  }
}

class ManualStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: SubStackProps) {
    super(scope, id, props);

    const runtimes = NODE_CDK_RUNTIMES;
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
        functionName: `${props.prefix}manual-instrumentation-${runtimeName}`,
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
  prefix: string;
}

class DockerizedStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: DockerizedStackProps) {
    super(scope, id, props);

    const account = process.env.CDK_DEFAULT_ACCOUNT;
    const region = process.env.CDK_DEFAULT_REGION;

    for (const runtime of ["python", "node", "java"]) {
      for (const architecture of [lambda.Architecture.X86_64, lambda.Architecture.ARM_64]) {
        const extensionImage = `${account}.dkr.ecr.${region}.amazonaws.com/${props.prefix}dash0-extension-${runtime}:latest`;
        const platform = architecture === lambda.Architecture.ARM_64
          ? ecr_assets.Platform.LINUX_ARM64
          : ecr_assets.Platform.LINUX_AMD64;
        new lambda.DockerImageFunction(this, `dockerized-${runtime}-${architecture.name}`, {
          functionName: `${props.prefix}dockerized-${runtime}-${architecture.name}`,
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

    const prefix = process.env.RESOURCE_PREFIX ?? '';

    const pythonLayer = getLatestLayerVersion(this, 'pythonLrapLayer', `${prefix}dash0-extension-python`);
    const nodeLayer = getLatestLayerVersion(this, 'nodeLrapLayer', `${prefix}dash0-extension-node`);
    const javaLayer = getLatestLayerVersion(this, 'javaLrapLayer', `${prefix}dash0-extension-java`);
    const manualLayer = getLatestLayerVersion(this, 'manualLrapLayer', `${prefix}dash0-extension-manual`);
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

    const dash0TokenSecret = new secretsmanager.Secret(this, 'Dash0TokenSecret', {
      secretName: `${prefix}dash0-token-secret`,
      secretStringValue: cdk.SecretValue.unsafePlainText(process.env.DASH0_DEV_API_TOKEN!),
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });
    dash0TokenSecret.grantRead(role);

    new PythonStack(this, 'PythonStack', {
      role,
      layer: pythonLayer,
      logGroup: sharedLogGroup,
      prefix,
      dash0TokenSecretArn: dash0TokenSecret.secretArn,
    });

    new NodeStack(this, 'NodeStack', {
      role,
      layer: nodeLayer,
      logGroup: sharedLogGroup,
      prefix,
      dash0TokenSecretArn: dash0TokenSecret.secretArn,
    });

    new JavaStack(this, 'JavaStack', {
      role,
      layer: javaLayer,
      logGroup: sharedLogGroup,
      prefix,
      dash0TokenSecretArn: dash0TokenSecret.secretArn,
    });

    new ManualStack(this, 'ManualStack', {
      role,
      layer: manualLayer,
      logGroup: sharedLogGroup,
      prefix,
    });

    new DockerizedStack(this, 'DockerizedStack', {
      role,
      logGroup: sharedLogGroup,
      prefix,
    });

    new PythonTracingScenariosStack(this, 'TracingScenariosStack', {
      layer: pythonLayer,
      logGroup: sharedLogGroup,
      prefix,
    });

    new NodeTracingScenariosStack(this, 'NodeTracingScenariosStack', {
      layer: nodeLayer,
      logGroup: sharedLogGroup,
      prefix,
    });

    new JavaTracingScenariosStack(this, 'JavaTracingScenariosStack', {
      layer: javaLayer,
      logGroup: sharedLogGroup,
      prefix,
    });

    new DbTestingStack(this, 'DbTestingStack', {
      nodeLayer,
      pythonLayer,
      logGroup: sharedLogGroup,
      prefix,
    });
  }
}

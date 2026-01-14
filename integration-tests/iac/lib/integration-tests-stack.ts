import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as path from 'path';

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
              DASH0_TOKEN: "auth_oEiAAAy5hZvVsEAADPm4uDyV7OcBmU4B",
              DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
              DASH0_EXTENSION_LOG_LEVEL: "info",
              SEND_ON_INVOCATION_END: invocationEnd,
            };
            if (traced === "false") {
              environment["DISABLE_AUTO_INSTRUMENTATION"] = "true";
            }
            if (scenario === "timeout") {
              environment["SLEEP_DURATION_MS"] = "20000";
            }

            const functionName = `${runtimeName}-${scenario}-${traced}-invocation-end-${invocationEnd}-${architecture.name}`;
            const handler = overrides?.handler ?? `${scenario}.handler`;
            const code = overrides?.code ?? lambda.Code.fromAsset(path.join(__dirname, '../lambdas'));
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

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup);
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

    createLambdas(this, [lambda.Runtime.JAVA_21], props.layer, props.role, props.logGroup, {
      handler: 'com.example.App::handleRequest',
      code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas/java21-success.jar')),
      memorySize: 512,
    });

    createLambdas(this, [lambda.Runtime.JAVA_17], props.layer, props.role, props.logGroup, {
      handler: 'com.example.App::handleRequest',
      code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas/java17-success.jar')),
      memorySize: 512,
    });
  }
}

export class IntegrationTestsStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const pythonLayer =
        lambda.LayerVersion.fromLayerVersionArn(this, 'pythonLrapLayer', 'arn:aws:lambda:us-west-2:285732642181:layer:lrap-python:68');
    const nodeLayer =
        lambda.LayerVersion.fromLayerVersionArn(this, 'nodeLrapLayer', 'arn:aws:lambda:us-west-2:285732642181:layer:lrap-node:56');
    const javaLayer =
        lambda.LayerVersion.fromLayerVersionArn(this, 'javaLrapLayer', 'arn:aws:lambda:us-west-2:285732642181:layer:lrap-java:27');
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
  }
}

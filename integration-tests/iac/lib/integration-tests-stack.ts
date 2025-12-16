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

function createLambdas(scope: Construct, runtimes: lambda.Runtime[], layer: lambda.ILayerVersion, role: iam.Role, logGroup: logs.ILogGroup) {
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
            const runtimeName = runtime.name.replace(/\./g, '-');
            const environment: any = {
              AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
              DASH0_TOKEN: "auth_oEiAAAy5hZvVsEAADPm4uDyV7OcBmU4B",
              LUMIGO_ENDPOINT: "http://127.0.0.1:9009/v1/traces",
              x_LUMIGO_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
              OTEL_EXTENSION_LOG_LEVEL: "info",
              SEND_ON_INVOCATION_END: invocationEnd,
              LUMIGO_TRACER_TOKEN: "t_xxxx",
            };
            if (traced === "false") {
              environment["DISABLE_AUTO_INSTRUMENTATION"] = "true";
            }

            const functionName = `${runtimeName}-${scenario}-${traced}-invocation-end-${invocationEnd}-${architecture.name}`;

            new lambda.Function(scope, functionName, {
              functionName: functionName,
              runtime: runtime,
              handler: `${scenario}.handler`,
              architecture: architecture,
              timeout: cdk.Duration.seconds(10),
              code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas')),
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
      lambda.Runtime.NODEJS_18_X,
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
    ];

    createLambdas(this, runtimes, props.layer, props.role, props.logGroup);
  }
}

export class IntegrationTestsStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const pythonLayer = lambda.LayerVersion.fromLayerVersionArn(this, 'pythonLrapLayer', 'arn:aws:lambda:us-west-2:285732642181:layer:lrap-python:5');
    const nodeLayer = lambda.LayerVersion.fromLayerVersionArn(this, 'nodeLrapLayer', 'arn:aws:lambda:us-west-2:285732642181:layer:lrap-node:34');
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
  }
}

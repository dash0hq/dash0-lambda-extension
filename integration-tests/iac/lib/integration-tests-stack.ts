import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as path from 'path';

export class IntegrationTestsStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const layer = lambda.LayerVersion.fromLayerVersionArn(this, 'LrapLayer', 'arn:aws:lambda:us-west-2:285732642181:layer:lrap:163');
    const role = new iam.Role(this, 'IntegrationTestsLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
      ],
    });

    for (let runtime of [lambda.Runtime.PYTHON_3_10, lambda.Runtime.PYTHON_3_11, lambda.Runtime.PYTHON_3_12, lambda.Runtime.PYTHON_3_13, lambda.Runtime.PYTHON_3_14]) {
      for (let architecture of [lambda.Architecture.X86_64, lambda.Architecture.ARM_64]) {
        for (let invocationEnd of ["true", "false"]) {
          for (let traced of ["true", "false"]) {
            for (let scenario of ["success", "timeout", "outofmemory", "importerror", "exception"]) {
              const runtimeName = runtime.name.replace(/\./g, '-');
              let environment: any = {
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
              new lambda.Function(this, `${runtimeName}-${scenario}-${traced}-invocation-end-${invocationEnd}-${architecture.name}`, {
                functionName: `${runtimeName}-${scenario}-${traced}-invocation-end-${invocationEnd}-${architecture.name}`,
                runtime: runtime,
                handler: `${scenario}.handler`,
                architecture: architecture,
                timeout: cdk.Duration.seconds(10),
                code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas')),
                layers: [layer],
                role,
                environment
              });
            }
          }
        }
      }
    }
  }
}

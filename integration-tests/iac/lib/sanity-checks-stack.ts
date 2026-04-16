import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as path from 'path';
import { createPythonCode } from './python-tracing-scenarios-stack';

export class SanityChecksStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const nodeLayerArn = process.env.SANITY_NODE_LAYER_ARN;
    const pythonLayerArn = process.env.SANITY_PYTHON_LAYER_ARN;

    if (!nodeLayerArn || !pythonLayerArn) {
      throw new Error('SANITY_NODE_LAYER_ARN and SANITY_PYTHON_LAYER_ARN environment variables are required');
    }

    const nodeLayer = lambda.LayerVersion.fromLayerVersionArn(this, 'NodeLayer', nodeLayerArn);
    const pythonLayer = lambda.LayerVersion.fromLayerVersionArn(this, 'PythonLayer', pythonLayerArn);

    const role = new iam.Role(this, 'SanityChecksLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
      ],
    });

    const logGroup = new logs.LogGroup(this, 'SanityChecksLogGroup', {
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      retention: logs.RetentionDays.ONE_DAY,
    });

    const environment = {
      AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
      DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
      DASH0_ENDPOINT: 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
      DASH0_EXTENSION_LOG_LEVEL: 'info',
      DASH0_SEND_ON_INVOCATION_END: 'true',
    };

    new lambda.Function(this, 'sanity-node-success', {
      functionName: 'sanity-node-success',
      runtime: lambda.Runtime.NODEJS_22_X,
      memorySize: 128,
      handler: 'success.handler',
      architecture: lambda.Architecture.X86_64,
      timeout: cdk.Duration.seconds(10),
      code: lambda.Code.fromAsset(path.join(__dirname, '../lambdas/node')),
      layers: [nodeLayer],
      role,
      environment,
      logGroup,
      loggingFormat: lambda.LoggingFormat.TEXT,
    });

    new lambda.Function(this, 'sanity-python-success', {
      functionName: 'sanity-python-success',
      runtime: lambda.Runtime.PYTHON_3_13,
      memorySize: 128,
      handler: 'success.handler',
      architecture: lambda.Architecture.X86_64,
      timeout: cdk.Duration.seconds(10),
      code: createPythonCode(),
      layers: [pythonLayer],
      role,
      environment,
      logGroup,
      loggingFormat: lambda.LoggingFormat.TEXT,
    });
  }
}

import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as lambdaNodejs from 'aws-cdk-lib/aws-lambda-nodejs';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as path from 'path';

export interface DbTestingStackProps extends cdk.NestedStackProps {
  layer: lambda.ILayerVersion;
  logGroup: logs.ILogGroup;
  prefix: string;
}

export class DbTestingStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: DbTestingStackProps) {
    super(scope, id, props);

    const prefix = props.prefix;

    const role = new iam.Role(this, 'DbTestingLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
      ],
    });

    const baseEnvironment = {
      AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
      DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
      DASH0_ENDPOINT: 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
      DASH0_EXTENSION_LOG_LEVEL: 'info',
    };

    const runtimes = [
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
    ];

    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');

      new lambdaNodejs.NodejsFunction(this, `RdsPostgresLambda-${runtimeName}`, {
        functionName: `${prefix}db-testing-rds-postgres-${runtimeName}`,
        runtime,
        entry: path.join(__dirname, '../lambdas/node/rds_postgres.mjs'),
        handler: 'handler',
        memorySize: 128,
        timeout: cdk.Duration.seconds(30),
        layers: [props.layer],
        role,
        bundling: {
          nodeModules: ['pg'],
        },
        environment: {
          ...baseEnvironment,
          DB_HOST: process.env.DB_HOST!,
          DB_PORT: process.env.DB_PORT || '5432',
          DB_NAME: process.env.DB_NAME || 'testdb',
          DB_USER: process.env.DB_USER!,
          DB_PASSWORD: process.env.DB_PASSWORD!,
        },
        logGroup: props.logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

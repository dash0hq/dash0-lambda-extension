import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as lambdaNodejs from 'aws-cdk-lib/aws-lambda-nodejs';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as rds from 'aws-cdk-lib/aws-rds';
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

    const vpc = new ec2.Vpc(this, 'DbTestingVpc', {
      vpcName: `${prefix}db-testing-vpc`,
      maxAzs: 2,
      natGateways: 1,
    });

    const lambdaSg = new ec2.SecurityGroup(this, 'LambdaSg', {
      vpc,
      description: 'Security group for DB testing lambdas',
    });

    const rdsSg = new ec2.SecurityGroup(this, 'RdsSg', {
      vpc,
      description: 'Security group for RDS instance',
    });
    rdsSg.addIngressRule(lambdaSg, ec2.Port.tcp(5432), 'Allow Lambda access to PostgreSQL');

    const dbInstance = new rds.DatabaseInstance(this, 'PostgresInstance', {
      instanceIdentifier: `${prefix}db-testing-postgres`,
      engine: rds.DatabaseInstanceEngine.postgres({
        version: rds.PostgresEngineVersion.VER_16,
      }),
      instanceType: ec2.InstanceType.of(ec2.InstanceClass.T3, ec2.InstanceSize.MICRO),
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      securityGroups: [rdsSg],
      databaseName: 'testdb',
      credentials: rds.Credentials.fromGeneratedSecret('postgres'),
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      deletionProtection: false,
    });

    const role = new iam.Role(this, 'DbTestingLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaVPCAccessExecutionRole'),
      ],
    });

    dbInstance.secret!.grantRead(role);

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
        vpc,
        vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
        securityGroups: [lambdaSg],
        layers: [props.layer],
        role,
        bundling: {
          nodeModules: ['pg'],
        },
        environment: {
          ...baseEnvironment,
          DB_HOST: dbInstance.dbInstanceEndpointAddress,
          DB_PORT: dbInstance.dbInstanceEndpointPort,
          DB_NAME: 'testdb',
          DB_SECRET_ARN: dbInstance.secret!.secretArn,
        },
        logGroup: props.logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

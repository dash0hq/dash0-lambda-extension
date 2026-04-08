import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as lambdaNodejs from 'aws-cdk-lib/aws-lambda-nodejs';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as path from 'path';
import { getLatestLayerVersion, importSharedResources } from './shared-resources-stack';

export class DbTestingStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const prefix = process.env.RESOURCE_PREFIX ?? '';
    const { role, logGroup } = importSharedResources(this);
    const nodeLayer = getLatestLayerVersion(this, 'nodeLayer', `${prefix}dash0-extension-node`);
    const pythonLayer = getLatestLayerVersion(this, 'pythonLayer', `${prefix}dash0-extension-python`);

    const baseEnvironment = {
      AWS_LAMBDA_EXEC_WRAPPER: '/opt/wrapper',
      DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
      DASH0_ENDPOINT: 'https://ingress.eu-west-1.aws.dash0-dev.com:4318',
      DASH0_EXTENSION_LOG_LEVEL: 'info',
    };

    const postgresEnv = {
      DB_HOST: 'shared-db-testing-postgres.czc66so6029n.eu-central-1.rds.amazonaws.com',
      DB_PORT: '5432',
      DB_NAME: 'testdb',
      DB_USER: 'postgres',
      DB_PASSWORD: process.env.TEST_POSTGRESS_PASSWORD!,
    };

    const mysqlEnv = {
      DB_HOST: 'shared-db-testing-mysql.czc66so6029n.eu-central-1.rds.amazonaws.com',
      DB_PORT: '3306',
      DB_NAME: 'testdb',
      DB_USER: 'admin',
      DB_PASSWORD: process.env.TEST_MYSQL_PASSWORD!,
    };

    // --- Node.js lambdas ---

    const nodeRuntimes = [
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
    ];

    for (const runtime of nodeRuntimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');

      new lambdaNodejs.NodejsFunction(this, `RdsPostgresLambda-${runtimeName}`, {
        functionName: `${prefix}db-testing-rds-postgres-${runtimeName}`,
        runtime,
        entry: path.join(__dirname, '../lambdas/node/rds_postgres.mjs'),
        handler: 'handler',
        memorySize: 128,
        timeout: cdk.Duration.seconds(30),
        layers: [nodeLayer],
        role,
        bundling: {
          nodeModules: ['pg'],
        },
        environment: {
          ...baseEnvironment,
          ...postgresEnv,
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });

      new lambdaNodejs.NodejsFunction(this, `RdsMysqlLambda-${runtimeName}`, {
        functionName: `${prefix}db-testing-rds-mysql-${runtimeName}`,
        runtime,
        entry: path.join(__dirname, '../lambdas/node/rds_mysql.mjs'),
        handler: 'handler',
        memorySize: 128,
        timeout: cdk.Duration.seconds(30),
        layers: [nodeLayer],
        role,
        bundling: {
          nodeModules: ['mysql2'],
        },
        environment: {
          ...baseEnvironment,
          ...mysqlEnv,
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }

    // --- Python lambdas ---

    const pythonRuntimes = [
      lambda.Runtime.PYTHON_3_12,
      lambda.Runtime.PYTHON_3_13,
      lambda.Runtime.PYTHON_3_14,
    ];

    for (const runtime of pythonRuntimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');
      const pythonVersion = runtime.name.replace('python', '');

      const pythonDbCode = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/python-db'), {
        assetHashType: cdk.AssetHashType.OUTPUT,
        bundling: {
          image: runtime.bundlingImage,
          command: [
            'bash', '-c',
            `pip install --no-cache-dir --platform manylinux2014_x86_64 --only-binary=:all: --python-version ${pythonVersion} -r requirements.txt -t /asset-output && cp -au . /asset-output`,
          ],
        },
      });

      new lambda.Function(this, `PythonRdsPostgresLambda-${runtimeName}`, {
        functionName: `${prefix}db-testing-rds-postgres-${runtimeName}`,
        runtime,
        handler: 'rds_postgres.handler',
        code: pythonDbCode,
        memorySize: 128,
        timeout: cdk.Duration.seconds(30),
        layers: [pythonLayer],
        role,
        environment: {
          ...baseEnvironment,
          ...postgresEnv,
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });

      new lambda.Function(this, `PythonRdsMysqlLambda-${runtimeName}`, {
        functionName: `${prefix}db-testing-rds-mysql-${runtimeName}`,
        runtime,
        handler: 'rds_mysql.handler',
        code: pythonDbCode,
        memorySize: 128,
        timeout: cdk.Duration.seconds(30),
        layers: [pythonLayer],
        role,
        environment: {
          ...baseEnvironment,
          ...mysqlEnv,
        },
        logGroup,
        loggingFormat: lambda.LoggingFormat.TEXT,
      });
    }
  }
}

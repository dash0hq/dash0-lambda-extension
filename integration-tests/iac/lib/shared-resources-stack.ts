import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as cr from 'aws-cdk-lib/custom-resources';

export function getLatestLayerVersion(scope: Construct, id: string, layerName: string): lambda.ILayerVersion {
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

export function importSharedResources(scope: cdk.Stack) {
  const roleArn = cdk.Fn.importValue('SharedResources-RoleArn');
  const role = iam.Role.fromRoleArn(scope, 'SharedRole', roleArn);

  const logGroupArn = cdk.Fn.importValue('SharedResources-LogGroupArn');
  const logGroup = logs.LogGroup.fromLogGroupArn(scope, 'SharedLogGroup', logGroupArn);

  const secretArn = cdk.Fn.importValue('SharedResources-SecretArn');

  return { role, logGroup, secretArn };
}

export class SharedResourcesStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const role = new iam.Role(this, 'IntegrationTestsLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSQSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSNSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonKinesisFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonEventBridgeFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonS3FullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AWSLambda_FullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AWSXRayDaemonWriteAccess'),
      ],
    });

    const logGroup = new logs.LogGroup(this, 'IntegrationTestsLogGroup', {
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      retention: logs.RetentionDays.ONE_DAY,
    });

    const secret = new secretsmanager.Secret(this, 'Dash0TokenSecret', {
      secretName: 'dash0-token-secret',
      secretStringValue: cdk.SecretValue.unsafePlainText(process.env.DASH0_DEV_API_TOKEN!),
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });
    secret.grantRead(role);

    new cdk.CfnOutput(this, 'RoleArn', {
      value: role.roleArn,
      exportName: 'SharedResources-RoleArn',
    });
    new cdk.CfnOutput(this, 'LogGroupArn', {
      value: logGroup.logGroupArn,
      exportName: 'SharedResources-LogGroupArn',
    });
    new cdk.CfnOutput(this, 'SecretArn', {
      value: secret.secretArn,
      exportName: 'SharedResources-SecretArn',
    });
  }
}

import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as rds from 'aws-cdk-lib/aws-rds';

export class SharedDbStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const vpc = new ec2.Vpc(this, 'SharedDbVpc', {
      vpcName: 'shared-db-vpc',
      maxAzs: 2,
      natGateways: 0,
    });

    const rdsSg = new ec2.SecurityGroup(this, 'RdsSg', {
      vpc,
      description: 'Security group for shared RDS instance',
    });
    rdsSg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(5432), 'Allow public PostgreSQL access');

    const dbInstance = new rds.DatabaseInstance(this, 'PostgresInstance', {
      instanceIdentifier: 'shared-db-testing-postgres',
      engine: rds.DatabaseInstanceEngine.postgres({
        version: rds.PostgresEngineVersion.VER_16,
      }),
      instanceType: ec2.InstanceType.of(ec2.InstanceClass.T3, ec2.InstanceSize.MICRO),
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC },
      publiclyAccessible: true,
      securityGroups: [rdsSg],
      databaseName: 'testdb',
      credentials: rds.Credentials.fromGeneratedSecret('postgres'),
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      deletionProtection: false,
    });

    new cdk.CfnOutput(this, 'DbHost', {
      value: dbInstance.dbInstanceEndpointAddress,
    });
    new cdk.CfnOutput(this, 'DbPort', {
      value: dbInstance.dbInstanceEndpointPort,
    });
    new cdk.CfnOutput(this, 'DbSecretArn', {
      value: dbInstance.secret!.secretArn,
    });
  }
}

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
      description: 'Security group for shared RDS instances',
    });
    rdsSg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(5432), 'Allow public PostgreSQL access');
    rdsSg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(3306), 'Allow public MySQL access');

    const postgresInstance = new rds.DatabaseInstance(this, 'PostgresInstance', {
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

    const mysqlInstance = new rds.DatabaseInstance(this, 'MysqlInstance', {
      instanceIdentifier: 'shared-db-testing-mysql',
      engine: rds.DatabaseInstanceEngine.mysql({
        version: rds.MysqlEngineVersion.VER_8_0,
      }),
      instanceType: ec2.InstanceType.of(ec2.InstanceClass.T3, ec2.InstanceSize.MICRO),
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC },
      publiclyAccessible: true,
      securityGroups: [rdsSg],
      databaseName: 'testdb',
      credentials: rds.Credentials.fromGeneratedSecret('admin'),
      removalPolicy: cdk.RemovalPolicy.DESTROY,
      deletionProtection: false,
    });

    new cdk.CfnOutput(this, 'PostgresHost', {
      value: postgresInstance.dbInstanceEndpointAddress,
    });
    new cdk.CfnOutput(this, 'PostgresPort', {
      value: postgresInstance.dbInstanceEndpointPort,
    });
    new cdk.CfnOutput(this, 'PostgresSecretArn', {
      value: postgresInstance.secret!.secretArn,
    });

    new cdk.CfnOutput(this, 'MysqlHost', {
      value: mysqlInstance.dbInstanceEndpointAddress,
    });
    new cdk.CfnOutput(this, 'MysqlPort', {
      value: mysqlInstance.dbInstanceEndpointPort,
    });
    new cdk.CfnOutput(this, 'MysqlSecretArn', {
      value: mysqlInstance.secret!.secretArn,
    });
  }
}

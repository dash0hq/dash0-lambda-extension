import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as sqs from 'aws-cdk-lib/aws-sqs';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as sns_subscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import * as lambda_event_sources from 'aws-cdk-lib/aws-lambda-event-sources';
import * as path from 'path';

export function createJavaCode(): lambda.Code {
  return lambda.Code.fromAsset(path.join(__dirname, '../lambdas/java'), {
    bundling: {
      image: lambda.Runtime.JAVA_17.bundlingImage,
      command: [
        'bash', '-c',
        'chmod +x gradlew && ./gradlew buildZip && cd /asset-output && jar xf /asset-input/build/distributions/lambda-java-1.0-SNAPSHOT.zip',
      ],
    },
  });
}

export interface JavaTracingScenariosStackProps extends cdk.NestedStackProps {
  layer: lambda.ILayerVersion;
  logGroup: logs.ILogGroup;
}

export class JavaTracingScenariosStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: JavaTracingScenariosStackProps) {
    super(scope, id, props);

    const role = new iam.Role(this, 'JavaTracingScenariosLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSQSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSNSFullAccess'),
      ],
    });

    const javaCode = createJavaCode();
    const baseEnvironment = {
      AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
      DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
      DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
      DASH0_EXTENSION_LOG_LEVEL: "info",
      DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER: "true",
    };
    const runtimes = [
      lambda.Runtime.JAVA_17,
      lambda.Runtime.JAVA_21,
      lambda.Runtime.JAVA_25,
    ];
    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');

      // Scenario 1: Lambda > SQS > Lambda
      const sqsQueue = new sqs.Queue(this, `TracingTestSqsQueue-${runtimeName}`, {
        queueName: `tracing-test-sqs-queue-${runtimeName}`,
        visibilityTimeout: cdk.Duration.seconds(30),
      });

      const sqsProducer = new lambda.Function(this, `SqsProducerLambda-${runtimeName}`, {
        functionName: `tracing-sqs-producer-${runtimeName}`,
        runtime,
        handler: 'org.example.SqsProducerHandler::handleRequest',
        code: javaCode,
        layers: [props.layer],
        role,
        memorySize: 512,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          QUEUE_URL: sqsQueue.queueUrl,
        },
      });

      const sqsConsumer = new lambda.Function(this, `SqsConsumerLambda-${runtimeName}`, {
        functionName: `tracing-sqs-consumer-${runtimeName}`,
        runtime,
        handler: 'org.example.ConsumerHandler::handleRequest',
        code: javaCode,
        layers: [props.layer],
        role,
        memorySize: 512,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      sqsConsumer.addEventSource(new lambda_event_sources.SqsEventSource(sqsQueue, {
        batchSize: 1,
      }));

      // Scenario 2: Lambda > SNS > Lambda
      const snsTopic = new sns.Topic(this, `TracingTestSnsTopic-${runtimeName}`, {
        topicName: `tracing-test-sns-topic-${runtimeName}`,
      });

      const snsProducer = new lambda.Function(this, `SnsProducerLambda-${runtimeName}`, {
        functionName: `tracing-sns-producer-${runtimeName}`,
        runtime,
        handler: 'org.example.SnsProducerHandler::handleRequest',
        code: javaCode,
        layers: [props.layer],
        role,
        memorySize: 512,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          TOPIC_ARN: snsTopic.topicArn,
        },
      });

      const snsConsumer = new lambda.Function(this, `SnsConsumerLambda-${runtimeName}`, {
        functionName: `tracing-sns-consumer-${runtimeName}`,
        runtime,
        handler: 'org.example.SnsConsumerHandler::handleRequest',
        code: javaCode,
        layers: [props.layer],
        role,
        memorySize: 512,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      snsTopic.addSubscription(new sns_subscriptions.LambdaSubscription(snsConsumer));

      // Scenario 3: Lambda > SNS > SQS > Lambda
      const snsSqsTopic = new sns.Topic(this, `TracingTestSnsSqsTopic-${runtimeName}`, {
        topicName: `tracing-test-sns-sqs-topic-${runtimeName}`,
      });

      const snsSqsQueue = new sqs.Queue(this, `TracingTestSnsSqsQueue-${runtimeName}`, {
        queueName: `tracing-test-sns-sqs-queue-${runtimeName}`,
        visibilityTimeout: cdk.Duration.seconds(30),
      });
      snsSqsTopic.addSubscription(new sns_subscriptions.SqsSubscription(snsSqsQueue, {
        rawMessageDelivery: false, // Keep SNS envelope to preserve MessageAttributes
      }));

      const snsSqsProducer = new lambda.Function(this, `SnsSqsProducerLambda-${runtimeName}`, {
        functionName: `tracing-sns-sqs-producer-${runtimeName}`,
        runtime,
        handler: 'org.example.SnsProducerHandler::handleRequest',
        code: javaCode,
        layers: [props.layer],
        role,
        memorySize: 512,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          TOPIC_ARN: snsSqsTopic.topicArn,
        },
      });

      const snsSqsConsumer = new lambda.Function(this, `SnsSqsConsumerLambda-${runtimeName}`, {
        functionName: `tracing-sns-sqs-consumer-${runtimeName}`,
        runtime,
        handler: 'org.example.ConsumerHandler::handleRequest',
        code: javaCode,
        layers: [props.layer],
        role,
        memorySize: 512,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      snsSqsConsumer.addEventSource(new lambda_event_sources.SqsEventSource(snsSqsQueue, {
        batchSize: 1,
      }));

    }
  }
}

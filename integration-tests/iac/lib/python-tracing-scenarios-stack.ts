import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as sqs from 'aws-cdk-lib/aws-sqs';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as sns_subscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import * as kinesis from 'aws-cdk-lib/aws-kinesis';
import * as events from 'aws-cdk-lib/aws-events';
import * as events_targets from 'aws-cdk-lib/aws-events-targets';
import * as lambda_event_sources from 'aws-cdk-lib/aws-lambda-event-sources';
import * as path from 'path';

export function createPythonCode(): lambda.Code {
  return lambda.Code.fromAsset(path.join(__dirname, '../lambdas/python'), {
    bundling: {
      image: lambda.Runtime.PYTHON_3_12.bundlingImage,
      command: [
        'bash', '-c',
        'pip install -r requirements.txt -t /asset-output && cp -au . /asset-output'
      ],
    },
  });
}

export interface PythonTracingScenariosStackProps extends cdk.NestedStackProps {
  layer: lambda.ILayerVersion;
  logGroup: logs.ILogGroup;
  prefix: string;
}

export class PythonTracingScenariosStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: PythonTracingScenariosStackProps) {
    super(scope, id, props);

    const role = new iam.Role(this, 'TracingScenariosLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSQSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSNSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonKinesisFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonEventBridgeFullAccess'),
      ],
    });

    const pythonCode = createPythonCode();
    const baseEnvironment = {
      AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
      DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
      DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
      DASH0_EXTENSION_LOG_LEVEL: "info",
      DASH0_EXTRACT_SPAN_LINKS_IN_CONSUMER: "true",
    };
    const runtimes = [
      lambda.Runtime.PYTHON_3_11,
      lambda.Runtime.PYTHON_3_12,
      lambda.Runtime.PYTHON_3_13,
      lambda.Runtime.PYTHON_3_14,
    ];
    const prefix = props.prefix;
    for (const runtime of runtimes) {
      const runtimeName = runtime.name.replace(/\./g, '-');

      // Scenario 1: Lambda > SQS > Lambda
      const sqsQueue = new sqs.Queue(this, `TracingTestSqsQueue-${runtimeName}`, {
        queueName: `${prefix}tracing-test-sqs-queue-${runtimeName}`,
        visibilityTimeout: cdk.Duration.seconds(30),
      });

      const sqsProducer = new lambda.Function(this, `SqsProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-sqs-producer-${runtimeName}`,
        runtime,
        handler: 'sqs_producer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          QUEUE_URL: sqsQueue.queueUrl,
        },
      });

      const sqsConsumer = new lambda.Function(this, `SqsConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-sqs-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      sqsConsumer.addEventSource(new lambda_event_sources.SqsEventSource(sqsQueue, {
        batchSize: 1,
      }));

      // Scenario 2: Lambda > SNS > Lambda
      const snsTopic = new sns.Topic(this, `TracingTestSnsTopic-${runtimeName}`, {
        topicName: `${prefix}tracing-test-sns-topic-${runtimeName}`,
      });

      const snsProducer = new lambda.Function(this, `SnsProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-sns-producer-${runtimeName}`,
        runtime,
        handler: 'sns_producer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          TOPIC_ARN: snsTopic.topicArn,
        },
      });

      const snsConsumer = new lambda.Function(this, `SnsConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-sns-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      snsTopic.addSubscription(new sns_subscriptions.LambdaSubscription(snsConsumer));

      // Scenario 3: Lambda > SNS > SQS > Lambda
      const snsSqsTopic = new sns.Topic(this, `TracingTestSnsSqsTopic-${runtimeName}`, {
        topicName: `${prefix}tracing-test-sns-sqs-topic-${runtimeName}`,
      });

      const snsSqsQueue = new sqs.Queue(this, `TracingTestSnsSqsQueue-${runtimeName}`, {
        queueName: `${prefix}tracing-test-sns-sqs-queue-${runtimeName}`,
        visibilityTimeout: cdk.Duration.seconds(30),
      });
      snsSqsTopic.addSubscription(new sns_subscriptions.SqsSubscription(snsSqsQueue, {
        rawMessageDelivery: false, // Keep SNS envelope to preserve MessageAttributes
      }));

      const snsSqsProducer = new lambda.Function(this, `SnsSqsProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-sns-sqs-producer-${runtimeName}`,
        runtime,
        handler: 'sns_producer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          TOPIC_ARN: snsSqsTopic.topicArn,
        },
      });

      const snsSqsConsumer = new lambda.Function(this, `SnsSqsConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-sns-sqs-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      snsSqsConsumer.addEventSource(new lambda_event_sources.SqsEventSource(snsSqsQueue, {
        batchSize: 1,
      }));

      // Scenario 4: Lambda > Kinesis > Lambda
      const kinesisStream = new kinesis.Stream(this, `TracingTestKinesisStream-${runtimeName}`, {
        streamName: `${prefix}tracing-test-kinesis-stream-${runtimeName}`,
        shardCount: 1,
      });

      const kinesisProducer = new lambda.Function(this, `KinesisProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-kinesis-producer-${runtimeName}`,
        runtime,
        handler: 'kinesis_producer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          STREAM_NAME: kinesisStream.streamName,
        },
      });

      const kinesisConsumer = new lambda.Function(this, `KinesisConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-kinesis-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });
      kinesisConsumer.addEventSource(new lambda_event_sources.KinesisEventSource(kinesisStream, {
        startingPosition: lambda.StartingPosition.TRIM_HORIZON,
        batchSize: 1,
      }));

      // Scenario 5: Lambda > EventBridge > Lambda
      const eventBus = new events.EventBus(this, `TracingTestEventBus-${runtimeName}`, {
        eventBusName: `${prefix}tracing-test-event-bus-${runtimeName}`,
      });

      const eventBridgeConsumer = new lambda.Function(this, `EventBridgeConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-eventbridge-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });

      new events.Rule(this, `TracingTestEventBridgeRule-${runtimeName}`, {
        ruleName: `${prefix}tracing-test-eventbridge-rule-${runtimeName}`,
        eventBus,
        eventPattern: {
          source: ['tracing-tests.producer'],
          detailType: ['TestMessage'],
        },
        targets: [new events_targets.LambdaFunction(eventBridgeConsumer)],
      });

      const eventBridgeProducer = new lambda.Function(this, `EventBridgeProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-eventbridge-producer-${runtimeName}`,
        runtime,
        handler: 'eventbridge_producer.handler',
        code: pythonCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          EVENT_BUS_NAME: eventBus.eventBusName,
        },
      });

    }
  }
}

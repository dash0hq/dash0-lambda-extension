import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as sqs from 'aws-cdk-lib/aws-sqs';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as sns_subscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import * as kinesis from 'aws-cdk-lib/aws-kinesis';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as s3n from 'aws-cdk-lib/aws-s3-notifications';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import * as apigatewayv2 from 'aws-cdk-lib/aws-apigatewayv2';
import * as apigatewayv2_integrations from 'aws-cdk-lib/aws-apigatewayv2-integrations';
import * as events from 'aws-cdk-lib/aws-events';
import * as events_targets from 'aws-cdk-lib/aws-events-targets';
import * as lambda_event_sources from 'aws-cdk-lib/aws-lambda-event-sources';
import * as path from 'path';

export interface NodeTracingScenariosStackProps extends cdk.NestedStackProps {
  layer: lambda.ILayerVersion;
  logGroup: logs.ILogGroup;
  prefix: string;
}

export class NodeTracingScenariosStack extends cdk.NestedStack {
  constructor(scope: Construct, id: string, props: NodeTracingScenariosStackProps) {
    super(scope, id, props);

    const role = new iam.Role(this, 'NodeTracingScenariosLambdaRole', {
      assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('service-role/AWSLambdaBasicExecutionRole'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSQSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSNSFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonKinesisFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonEventBridgeFullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonS3FullAccess'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AWSLambda_FullAccess'),
      ],
    });

    const nodeCode = lambda.Code.fromAsset(path.join(__dirname, '../lambdas/node'));
    const baseEnvironment = {
      AWS_LAMBDA_EXEC_WRAPPER: "/opt/wrapper",
      DASH0_TOKEN: process.env.DASH0_DEV_API_TOKEN!,
      DASH0_ENDPOINT: "https://ingress.eu-west-1.aws.dash0-dev.com:4318",
      DASH0_EXTENSION_LOG_LEVEL: "info",
    };
    const runtimes = [
      lambda.Runtime.NODEJS_20_X,
      lambda.Runtime.NODEJS_22_X,
      lambda.Runtime.NODEJS_24_X,
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
        code: nodeCode,
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
        code: nodeCode,
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
        code: nodeCode,
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
        code: nodeCode,
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
        code: nodeCode,
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
        code: nodeCode,
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
        removalPolicy: cdk.RemovalPolicy.DESTROY,
      });

      const kinesisProducer = new lambda.Function(this, `KinesisProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-kinesis-producer-${runtimeName}`,
        runtime,
        handler: 'kinesis_producer.handler',
        code: nodeCode,
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
        code: nodeCode,
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
        code: nodeCode,
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
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          EVENT_BUS_NAME: eventBus.eventBusName,
        },
      });

      // Scenario 6: Lambda > API Gateway > Lambda
      const apiGatewayConsumer = new lambda.Function(this, `ApiGatewayConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-apigateway-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });

      const api = new apigateway.LambdaRestApi(this, `TracingTestApi-${runtimeName}`, {
        restApiName: `${prefix}tracing-test-api-${runtimeName}`,
        handler: apiGatewayConsumer,
        proxy: true,
      });

      const apiGatewayProducer = new lambda.Function(this, `ApiGatewayProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-apigateway-producer-${runtimeName}`,
        runtime,
        handler: 'apigateway_producer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          API_URL: api.url,
        },
      });

      // Scenario 6b: Lambda > HTTP API Gateway > Lambda
      const httpApiConsumer = new lambda.Function(this, `HttpApiConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-httpapi-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });

      const httpApi = new apigatewayv2.HttpApi(this, `TracingTestHttpApi-${runtimeName}`, {
        apiName: `${prefix}tracing-test-httpapi-${runtimeName}`,
        defaultIntegration: new apigatewayv2_integrations.HttpLambdaIntegration(
          `HttpApiIntegration-${runtimeName}`,
          httpApiConsumer,
        ),
      });

      const httpApiProducer = new lambda.Function(this, `HttpApiProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-httpapi-producer-${runtimeName}`,
        runtime,
        handler: 'apigateway_producer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          API_URL: httpApi.url!,
        },
      });

      // Scenario 7: Lambda > S3 > Lambda
      const s3Bucket = new s3.Bucket(this, `TracingTestS3Bucket-${runtimeName}`, {
        bucketName: `${prefix}tracing-test-s3-bucket-${runtimeName}`,
        removalPolicy: cdk.RemovalPolicy.DESTROY,
        autoDeleteObjects: true,
      });

      const s3Consumer = new lambda.Function(this, `S3ConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-s3-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });

      s3Bucket.addEventNotification(
        s3.EventType.OBJECT_CREATED,
        new s3n.LambdaDestination(s3Consumer),
        { prefix: 'test-events/' },
      );

      const s3Producer = new lambda.Function(this, `S3ProducerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-s3-producer-${runtimeName}`,
        runtime,
        handler: 's3_producer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          BUCKET_NAME: s3Bucket.bucketName,
        },
      });

      // Scenario 8: Lambda > Lambda
      const lambdaConsumer = new lambda.Function(this, `LambdaConsumerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-lambda-consumer-${runtimeName}`,
        runtime,
        handler: 'consumer.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: baseEnvironment,
      });

      const lambdaInvoker = new lambda.Function(this, `LambdaInvokerLambda-${runtimeName}`, {
        functionName: `${prefix}tracing-lambda-invoker-${runtimeName}`,
        runtime,
        handler: 'lambda_invoker.handler',
        code: nodeCode,
        layers: [props.layer],
        role,
        timeout: cdk.Duration.seconds(10),
        logGroup: props.logGroup,
        environment: {
          ...baseEnvironment,
          TARGET_FUNCTION_NAME: lambdaConsumer.functionName,
        },
      });

    }
  }
}

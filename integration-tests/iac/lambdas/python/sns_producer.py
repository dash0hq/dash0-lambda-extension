import json
import os
import boto3

sns = boto3.client('sns')


def handler(event, context):
    topic_arn = os.environ['TOPIC_ARN']

    message = {
        'message': 'Hello from SNS producer!',
        'request_id': context.aws_request_id,
    }

    response = sns.publish(
        TopicArn=topic_arn,
        Message=json.dumps(message),
        Subject='Test Message',
    )

    print(f"Published message to SNS: {response['MessageId']}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'message_id': response['MessageId'],
        }),
    }

import json
import os
import boto3

sqs = boto3.client('sqs')


def handler(event, context):
    queue_url = os.environ['QUEUE_URL']

    message = {
        'message': 'Hello from SQS producer!',
        'request_id': context.aws_request_id,
    }

    response = sqs.send_message(
        QueueUrl=queue_url,
        MessageBody=json.dumps(message),
    )

    print(f"Sent message to SQS: {response['MessageId']}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'message_id': response['MessageId'],
        }),
    }

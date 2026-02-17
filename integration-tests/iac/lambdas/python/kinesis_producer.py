import json
import os
import boto3

kinesis = boto3.client('kinesis')


def handler(event, context):
    stream_name = os.environ['STREAM_NAME']

    message = {
        'message': 'Hello from Kinesis producer!',
        'request_id': context.aws_request_id,
    }

    response = kinesis.put_record(
        StreamName=stream_name,
        Data=json.dumps(message),
        PartitionKey=context.aws_request_id,
    )

    print(f"Put record to Kinesis: {response['SequenceNumber']}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'sequence_number': response['SequenceNumber'],
        }),
    }

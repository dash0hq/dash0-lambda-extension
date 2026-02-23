import json
import os
import boto3

s3 = boto3.client('s3')


def handler(event, context):
    bucket_name = os.environ['BUCKET_NAME']

    body = {
        'message': 'Hello from S3 producer!',
        'requestId': context.aws_request_id,
    }

    key = f'test-events/{context.aws_request_id}.json'

    response = s3.put_object(
        Bucket=bucket_name,
        Key=key,
        Body=json.dumps(body),
        ContentType='application/json',
    )

    print(f"Put object to S3: {key}, ETag: {response['ETag']}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'key': key,
            'etag': response['ETag'],
        }),
    }

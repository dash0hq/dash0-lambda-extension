import json
import os
import boto3

lambda_client = boto3.client('lambda')


def handler(event, context):
    target_function_name = os.environ['TARGET_FUNCTION_NAME']

    payload = {
        'message': 'Hello from Lambda invoker!',
        'request_id': context.aws_request_id,
    }

    response = lambda_client.invoke(
        FunctionName=target_function_name,
        InvocationType='RequestResponse',
        Payload=json.dumps(payload),
    )

    response_payload = json.loads(response['Payload'].read())
    print(f"Invoked Lambda {target_function_name}, status: {response['StatusCode']}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'invoked_function': target_function_name,
            'status_code': response['StatusCode'],
            'response': response_payload,
        }),
    }

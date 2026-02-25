import json
import os
import requests


def handler(event, context):
    api_url = os.environ['API_URL']

    payload = {
        'message': 'Hello from API Gateway producer!',
        'requestId': context.aws_request_id,
    }

    response = requests.post(api_url, json=payload)

    print(f"API Gateway response: {response.status_code} {response.text}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'api_status_code': response.status_code,
            'api_response': response.text,
        }),
    }

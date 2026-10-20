import json
import logging
import requests

def handler(event, context):
    payload = {
      "title": 'foo',
      "current_lambda": context.function_name,
      "body": 'bar',
      "userId": 1,
      # Masked by the Python distro via DASH0_MASK_RULES, set on the function by the stack.
      "masked_field": "this should not be seen!",
    }
    response = requests.post(
        "https://jsonplaceholder.typicode.com/posts",
        json=payload,
        headers={"x-masked-field": "this should not be seen!"},
    )

    print(f"response.status_code: {response.status_code}")
    logging.warning("let's parse this as a warning")

    return {
        'statusCode': 200,
        'body': json.dumps('Hello from Lambda!')
    }

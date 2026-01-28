import json
import requests

def handler(event, context):
    payload = {
      "title": 'foo',
      "current_lambda": context.function_name,
      "body": 'bar',
      "userId": 1,
    }
    response = requests.post("https://jsonplaceholder.typicode.com/posts", json=payload)

    print(f"response.status_code: {response.status_code}")

    print(event['non_existent_key'])  # This will raise a KeyError
    
    return {
        'statusCode': 200,
        'body': json.dumps('Hello from Lambda!')
    }

import json
import requests
import time

def handler(event, context):
    payload = {
      "title": 'foo',
      "current_lambda": context.function_name,
      "body": 'bar',
      "userId": 1,
    }
    response = requests.post("https://jsonplaceholder.typicode.com/posts", json=payload)

    print(f"response.status_code: {response.status_code}")
    time.sleep(50)
    
    return {
        'statusCode': 200,
        'body': json.dumps('Hello from Lambda!')
    }

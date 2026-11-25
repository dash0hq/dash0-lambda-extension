import json
import logging
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
    logging.warning("let's parse this as a warning")
    # print() skips the runtime's level prefix, so only the JSON `level` field carries severity.
    print(json.dumps({"level": "error", "jsonSeverity": "inferred"}))
    # The logging method sets the level (as CloudWatch shows it); a JSON `level` inside is ignored.
    logging.error(json.dumps({"level": "info", "loggingMethod": "error"}))

    return {
        'statusCode': 200,
        'body': json.dumps('Hello from Lambda!')
    }

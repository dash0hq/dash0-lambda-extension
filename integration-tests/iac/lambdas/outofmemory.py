import json
import requests
import time

def handler(event, context):
    time.sleep(1)
    print("response.status_code: 200")

    x = "x"
    try:
        while True:
            x = x + x  # doubles approximately every iteration
#             print(f"Size: {len(x) / (1024**2):.2f} MB")
    except Exception as e:
        print("Exception:", e)
    
    return {
        'statusCode': 200,
        'body': json.dumps('Hello from Lambda!')
    }

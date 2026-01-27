import json
import doesnt_exist

def handler(event, context):

    print(event)
    
    return {
        'statusCode': 200,
        'body': json.dumps('Hello from Lambda!')
    }

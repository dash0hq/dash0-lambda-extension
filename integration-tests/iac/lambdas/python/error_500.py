import json


def handler(event, context):
    return {
        'statusCode': 500,
        'body': json.dumps({
            'error': 'Internal Server Error',
        }),
    }

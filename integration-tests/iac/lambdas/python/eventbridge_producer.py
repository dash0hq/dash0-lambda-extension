import json
import os
import boto3

events = boto3.client('events')


def handler(event, context):
    event_bus_name = os.environ['EVENT_BUS_NAME']

    detail = {
        'message': 'Hello from EventBridge producer!',
        'requestId': context.aws_request_id,
    }

    response = events.put_events(
        Entries=[
            {
                'Source': 'tracing-tests.producer',
                'DetailType': 'TestMessage',
                'Detail': json.dumps(detail),
                'EventBusName': event_bus_name,
            }
        ]
    )

    print(f"Put event to EventBridge: {response}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'failed_entry_count': response['FailedEntryCount'],
        }),
    }

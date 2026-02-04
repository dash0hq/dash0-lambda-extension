import json


def handler(event, context):
    print(f"Received event: {json.dumps(event)}")

    # Check if this is an EventBridge event (has detail-type field)
    if 'detail-type' in event:
        print(f"EventBridge event from source: {event.get('source')}")
        print(f"Detail type: {event.get('detail-type')}")
        print(f"Detail: {json.dumps(event.get('detail', {}))}")
        return {
            'statusCode': 200,
            'body': json.dumps({
                'event_type': 'eventbridge',
                'source': event.get('source'),
                'detail_type': event.get('detail-type'),
            }),
        }

    # Handle Records-based events (SQS, SNS)
    records = event.get('Records', [])
    print(f"Processing {len(records)} record(s)")

    for i, record in enumerate(records):
        print(f"Record {i}: {json.dumps(record)}")

    return {
        'statusCode': 200,
        'body': json.dumps({
            'records_processed': len(records),
        }),
    }

import json


def handler(event, context):
    print(f"Received event: {json.dumps(event)}")

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

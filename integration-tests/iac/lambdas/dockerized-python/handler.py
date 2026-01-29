def handler(event, context):
    print(f"event payload: {event}")
    return {
        "statusCode": 200,
        "body": '{"message":"Success"}'
    }

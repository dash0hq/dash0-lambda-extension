import json


def handler(event, context):
    print(f"Received event: {json.dumps(event)}")
    raise RuntimeError("Intentional error for retry testing")

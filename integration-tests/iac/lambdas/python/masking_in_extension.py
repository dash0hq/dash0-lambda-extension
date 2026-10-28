import boto3
import requests

# Payloads the Python distro does not mask for `requests`: the URL query, the request headers
# and the request body. Plus an AWS SDK request body, which the distro masks unless
# DASH0_MASK_IN_EXTENSION=true hands masking to the extension. The extension masks all of them
# with the default patterns.
SECRET = "XQZ-IT-SECRET"


def handler(event, context):
    response = requests.post(
        f"https://jsonplaceholder.typicode.com/posts?api_token={SECRET}&page=1",
        json={"email": "jane@example.com", "password": SECRET},
        headers={"x-api-key": SECRET},
    )
    try:
        # The table doesn't exist; the distro captures the request body before the call fails.
        boto3.client("dynamodb").get_item(
            TableName="masking-in-extension", Key={"apiToken": {"S": SECRET}}
        )
    except Exception:
        pass
    return {"statusCode": response.status_code}

import requests

# Payloads the Python distro does not mask for `requests`: the URL query, the request headers
# and the request body. With DASH0_MASK_IN_EXTENSION=true the extension masks all three with
# the default patterns.
SECRET = "XQZ-IT-SECRET"


def handler(event, context):
    response = requests.post(
        f"https://jsonplaceholder.typicode.com/posts?api_token={SECRET}&page=1",
        json={"email": "jane@example.com", "password": SECRET},
        headers={"x-api-key": SECRET},
    )
    return {"statusCode": response.status_code}

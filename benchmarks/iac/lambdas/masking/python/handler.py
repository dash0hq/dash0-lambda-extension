import json
import os

import requests


# Same body in every runtime: ~3.9 KB of compact JSON with three secret-looking keys.
def body(n):
    items = [
        {"sku": f"SKU-{i:04d}", "name": f"Item number {i} with a short description", "qty": i % 5 + 1, "priceCents": i * 100 + 99}
        for i in range(41)
    ]
    return json.dumps(
        {
            "orderId": n,
            "customer": {"name": "Jane Doe", "email": "jane@example.com", "password": f"pw-SECRETVALUE-{n}"},
            "auth": {"api_key": f"key-SECRETVALUE-{n}", "access_token": f"tok-SECRETVALUE-{n}"},
            "items": items,
        },
        separators=(",", ":"),
    )


session = requests.Session()


def handler(event, context):
    echoed = 0
    for n in range(5):
        r = session.post(os.environ["ECHO_URL"], data=body(n), headers={"content-type": "application/json"})
        echoed += len(r.text)
    return {"statusCode": 200, "body": json.dumps({"orderId": event.get("orderId"), "echoed": echoed})}

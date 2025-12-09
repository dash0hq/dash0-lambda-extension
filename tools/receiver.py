import base64
import gzip
import json
import time

from google.protobuf.json_format import MessageToDict
from opentelemetry.proto.collector.trace.v1.trace_service_pb2 import ExportTraceServiceRequest
from opentelemetry.proto.collector.logs.v1.logs_service_pb2 import (
    ExportLogsServiceRequest,
)

def lambda_handler(event, context):
    print(event)
    # time.sleep(10)

    headers = {k.lower(): v for k, v in (event.get("headers") or {}).items()}
    content_encoding = headers.get("content-encoding", "").lower()

    # Decode base64 body
    body_b64 = event.get("body", "")
    raw_bytes = base64.b64decode(body_b64) if event.get("isBase64Encoded", False) else body_b64.encode()

    # Decompress if gzip
    if content_encoding == "gzip":
        raw_bytes = gzip.decompress(raw_bytes)

    # Parse OTLP protobuf
    if event["rawPath"] == "/v1/traces":
        req = ExportTraceServiceRequest()
        req.ParseFromString(raw_bytes)
    else:
        req = ExportLogsServiceRequest()
        req.ParseFromString(raw_bytes)

    # Convert proto → JSON
    json_data = MessageToDict(
        req,
        preserving_proto_field_name=True,
    )

    # Log the JSON
    line = json.dumps(json_data, separators=(",", ":"))
    print("JSON:" + line)

    return {
        "statusCode": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json.dumps(json_data)
    }

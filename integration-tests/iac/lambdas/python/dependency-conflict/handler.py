from google.protobuf import __version__ as protobuf_version
from google.protobuf.struct_pb2 import Struct


def handler(event, context):
    print(f"protobuf version: {protobuf_version}")

    # Create a simple protobuf Struct
    s = Struct()
    s.update({"message": "Hello from protobuf!", "number": 42})

    return {
        "statusCode": 200,
        "body": f"protobuf version: {protobuf_version}, struct: {s}"
    }

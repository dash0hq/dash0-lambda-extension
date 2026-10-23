"""
Runs one known-masking-gap scenario and prints every exported span as JSON:
[{"name", "scope", "attributes", "events": [{"name", "attributes"}]}].

Usage: capture_gaps.py <scenario> <secret>

test_known_masking_gaps.py starts it once per scenario, in a fresh interpreter, with the
environment `map_masking_env` produces, the same way test_masking_env.py runs
capture_payloads.py. Each scenario drives a real library through the distro's hooks (or
the upstream OpenTelemetry instrumentation the distro installs) and puts `secret` into the
payload the gap is about.
"""

import json
import sqlite3
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Callable, Dict, Iterator, List
from urllib.parse import urlencode

# Sets the global tracer provider and exporter, and provides the app drivers
import capture_payloads as base

SECRET = sys.argv[2] if len(sys.argv) > 2 else "s3cr3t"


class _EchoHandler(BaseHTTPRequestHandler):
    """Answers every request with a small JSON body and a Set-Cookie header."""

    def _answer(self) -> None:
        self.rfile.read(int(self.headers.get("content-length", 0) or 0))
        body = b'{"ok": true}'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("set-cookie", f"session={SECRET}; HttpOnly")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = _answer

    def log_message(self, *args: Any) -> None:
        pass


def _local_server() -> Iterator[str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), _EchoHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()


def with_local_server(drive: Callable[[str], None]) -> None:
    for url in _local_server():
        drive(url)


# --- query strings in URL attributes ------------------------------------------------


def handler(event: Dict[str, Any], context: Any) -> Dict[str, Any]:
    """The Lambda handler AwsLambdaInstrumentor wraps (_HANDLER=capture_gaps.handler)."""
    return {"statusCode": 200, "body": "ok"}


class _LambdaContext:
    aws_request_id = "req-1"
    invoked_function_arn = "arn:aws:lambda:eu-central-1:123456789012:function:orders"
    function_name = "orders"
    function_version = "$LATEST"
    memory_limit_in_mb = 128
    log_group_name = log_stream_name = "orders"

    def get_remaining_time_in_millis(self) -> int:
        return 10_000


def lambda_api_gateway_query() -> None:
    import os

    from opentelemetry.instrumentation.aws_lambda import AwsLambdaInstrumentor

    os.environ["_HANDLER"] = "capture_gaps.handler"
    os.environ.setdefault("AWS_LAMBDA_FUNCTION_NAME", "orders")
    AwsLambdaInstrumentor().instrument()
    import capture_gaps  # the module the instrumentor wrapped, not __main__

    capture_gaps.handler(
        {
            "resource": "/orders",
            "path": "/orders",
            "httpMethod": "GET",
            "headers": {"Host": "api.example.com", "User-Agent": "curl"},
            "queryStringParameters": {"password": SECRET},
            "requestContext": {"httpMethod": "GET", "resourcePath": "/orders"},
        },
        _LambdaContext(),
    )


def _flask_app() -> Any:
    from flask import Flask, jsonify

    app = Flask("gaps")

    @app.route("/orders", methods=["GET", "POST"])
    def orders() -> Any:
        return jsonify({"ok": True}), 200, {"set-cookie": f"session={SECRET}"}

    @app.get("/fail")
    def fail() -> Any:
        raise ValueError(f"login failed for password {SECRET}")

    return app


def flask_query() -> None:
    _flask_app().test_client().get("/orders?" + urlencode({"password": SECRET}))


def fastapi_query() -> None:
    from fastapi import FastAPI
    from fastapi.testclient import TestClient

    app = FastAPI()

    @app.get("/orders")
    async def orders() -> Any:
        return {"ok": True}

    TestClient(app).get("/orders?" + urlencode({"password": SECRET}))


def django_query() -> None:
    from django.test import Client

    Client().get("/orders?" + urlencode({"password": SECRET}))


def requests_query() -> None:
    import requests

    with_local_server(
        lambda url: requests.get(url + "/orders?" + urlencode({"password": SECRET}))
    )


def httpx_query() -> None:
    import httpx

    with_local_server(
        lambda url: httpx.get(url + "/orders?" + urlencode({"password": SECRET}))
    )


# --- bodies that aren't JSON ---------------------------------------------------------


def requests_form_body() -> None:
    import requests

    with_local_server(
        lambda url: requests.post(url, data={"user": "jane", "password": SECRET})
    )


def requests_multipart_body() -> None:
    import requests

    with_local_server(
        lambda url: requests.post(
            url, files={"file": ("a.txt", b"x")}, data={"password": SECRET}
        )
    )


def httpx_form_body() -> None:
    import httpx

    with_local_server(lambda url: httpx.post(url, data={"password": SECRET}))


def httpx_text_body() -> None:
    import httpx

    with_local_server(
        lambda url: httpx.post(
            url,
            content=f"password: {SECRET}",
            headers={"content-type": "text/plain"},
        )
    )


def django_form_body() -> None:
    from django.test import Client

    Client().post("/orders", data={"user": "jane", "password": SECRET})


def fastapi_chunked_json_body() -> None:
    from fastapi import FastAPI, Request
    from fastapi.testclient import TestClient

    app = FastAPI()

    @app.post("/orders")
    async def orders(request: Request) -> Any:
        await request.body()
        return {"ok": True}

    body = json.dumps({"password": SECRET}).encode()
    # A streamed request body reaches the app as several http.request messages
    chunks = [body[:5], body[5:]]
    TestClient(app).post(
        "/orders",
        content=iter(chunks),
        headers={"content-type": "application/json"},
    )


# --- AWS SDK -------------------------------------------------------------------------


def _client(service: str) -> Any:
    import boto3

    return boto3.client(
        service,
        region_name="eu-central-1",
        aws_access_key_id="testing",
        aws_secret_access_key="testing",  # pragma: allowlist secret
    )


def ssm_parameter_value() -> None:
    from botocore.stub import Stubber

    ssm = _client("ssm")
    with Stubber(ssm) as stub:
        stub.add_response(
            "get_parameter",
            {
                "Parameter": {
                    "Name": "/orders/db",
                    "Type": "SecureString",
                    "Value": SECRET,
                }
            },
        )
        ssm.get_parameter(Name="/orders/db", WithDecryption=True)


def kms_plaintext() -> None:
    from botocore.stub import Stubber

    kms = _client("kms")
    with Stubber(kms) as stub:
        stub.add_response(
            "decrypt", {"KeyId": "k1", "Plaintext": SECRET.encode()}
        )
        kms.decrypt(CiphertextBlob=b"blob")


def dynamodb_expression_values() -> None:
    from botocore.stub import Stubber

    dynamodb = _client("dynamodb")
    with Stubber(dynamodb) as stub:
        stub.add_response("query", {"Items": [], "Count": 0})
        dynamodb.query(
            TableName="users",
            KeyConditionExpression="password = :pw",
            ExpressionAttributeValues={":pw": {"S": SECRET}},
        )


def sqs_message_body_json() -> None:
    from botocore.stub import Stubber

    sqs = _client("sqs")
    with Stubber(sqs) as stub:
        stub.add_response("send_message", {"MessageId": "m1"})
        sqs.send_message(
            QueueUrl=base.QUEUE_URL, MessageBody=json.dumps({"password": SECRET})
        )


def eventbridge_detail_json() -> None:
    from botocore.stub import Stubber

    events = _client("events")
    with Stubber(events) as stub:
        stub.add_response("put_events", {"FailedEntryCount": 0, "Entries": []})
        events.put_events(
            Entries=[
                {
                    "Source": "orders",
                    "DetailType": "login",
                    "Detail": json.dumps({"password": SECRET}),
                }
            ]
        )


# --- Redis ---------------------------------------------------------------------------


def redis_hgetall_bytes_keys() -> None:
    from opentelemetry.instrumentation.redis import RedisInstrumentor

    from dash0_opentelemetry.instrumentations.redis import instrumentor as redis

    hooks = base._hooks_of(RedisInstrumentor, redis.install_instrumentation)
    span = base.tracer.start_span("HGETALL")
    # redis-py returns bytes keys and values unless decode_responses=True
    hooks["response_hook"](span, None, {b"user": b"jane", b"password": SECRET.encode()})
    span.end()


def redis_statement() -> None:
    from opentelemetry.instrumentation.redis import RedisInstrumentor

    from dash0_opentelemetry.instrumentations.redis import instrumentor as redis

    hooks = base._hooks_of(RedisInstrumentor, redis.install_instrumentation)
    span = base.tracer.start_span("HSET")
    hooks["request_hook"](span, None, ["HSET", "user:1", "password", SECRET], {})
    span.end()


# --- databases -----------------------------------------------------------------------


def dbapi_statement_parameters() -> None:
    """
    The psycopg2 instrumentation forces capture_parameters=True on DatabaseApiIntegration,
    for every DB-API driver. sqlite3 stands in for the driver: psycopg2 and pymysql need a
    server, but go through the same DatabaseApiIntegration.
    """
    from opentelemetry.instrumentation.dbapi import wrap_connect

    from dash0_opentelemetry.instrumentations.psycopg2 import instrumentor as psycopg2

    psycopg2.install_instrumentation()
    wrap_connect(__name__, sqlite3, "connect", "sqlite")
    connection = sqlite3.connect(":memory:")
    connection.execute("CREATE TABLE users (name TEXT, password TEXT)")
    connection.cursor().execute(
        "INSERT INTO users (name, password) VALUES (?, ?)", ("jane", SECRET)
    )


def dbapi_inline_sql_values() -> None:
    from opentelemetry.instrumentation.dbapi import wrap_connect

    wrap_connect(__name__, sqlite3, "connect", "sqlite")
    connection = sqlite3.connect(":memory:")
    connection.execute("CREATE TABLE users (name TEXT, password TEXT)")
    connection.cursor().execute(
        f"INSERT INTO users (name, password) VALUES ('jane', '{SECRET}')"
    )


# --- gRPC ----------------------------------------------------------------------------


def grpc_payloads() -> None:
    from concurrent import futures

    import grpc

    from dash0_opentelemetry.instrumentations.grpcio import instrumentor as grpcio

    grpcio.install_instrumentation()

    def login(request: bytes, context: Any) -> bytes:
        return b"token=" + SECRET.encode()

    server = grpc.server(futures.ThreadPoolExecutor(max_workers=2))
    server.add_generic_rpc_handlers(
        (
            grpc.method_handlers_generic_handler(
                "auth.Auth", {"Login": grpc.unary_unary_rpc_method_handler(login)}
            ),
        )
    )
    port = server.add_insecure_port("127.0.0.1:0")
    server.start()
    try:
        with grpc.insecure_channel(f"127.0.0.1:{port}") as channel:
            channel.unary_unary("/auth.Auth/Login")(b"password=" + SECRET.encode())
    finally:
        server.stop(None)


# --- exceptions, cookies, opt-in header capture --------------------------------------


def flask_exception_message() -> None:
    _flask_app().test_client().get("/fail")


def requests_cookie_header() -> None:
    import requests

    with_local_server(
        lambda url: requests.get(url, headers={"cookie": f"session={SECRET}"})
    )


def flask_cookie_header() -> None:
    client = _flask_app().test_client()
    client.set_cookie("session", SECRET)
    client.get("/orders")


def flask_opt_in_header_capture() -> None:
    _flask_app().test_client().get(
        "/orders", headers={"authorization": f"Bearer {SECRET}"}
    )


SCENARIOS: Dict[str, Callable[[], None]] = {
    f.__name__: f
    for f in [
        lambda_api_gateway_query,
        flask_query,
        fastapi_query,
        django_query,
        requests_query,
        httpx_query,
        requests_form_body,
        requests_multipart_body,
        httpx_form_body,
        httpx_text_body,
        django_form_body,
        fastapi_chunked_json_body,
        ssm_parameter_value,
        kms_plaintext,
        dynamodb_expression_values,
        sqs_message_body_json,
        eventbridge_detail_json,
        redis_hgetall_bytes_keys,
        redis_statement,
        dbapi_statement_parameters,
        dbapi_inline_sql_values,
        grpc_payloads,
        flask_exception_message,
        requests_cookie_header,
        flask_cookie_header,
        flask_opt_in_header_capture,
    ]
}


def main() -> None:
    base.install()
    try:
        SCENARIOS[sys.argv[1]]()
    except Exception:
        # Errors raised by the app on purpose (flask_exception_message) are expected;
        # the spans are what matters.
        pass
    spans: List[Dict[str, Any]] = [
        {
            "name": span.name,
            "scope": span.instrumentation_scope.name if span.instrumentation_scope else "",
            "attributes": dict(span.attributes or {}),
            "events": [
                {"name": e.name, "attributes": dict(e.attributes or {})}
                for e in span.events
            ],
        }
        for span in base.exporter.get_finished_spans()
    ]
    json.dump(spans, sys.stdout, default=str)


if __name__ == "__main__":
    main()

"""
Captures the payload attributes the distro sets, with the masking environment this process
was started with, and prints them as JSON: {target: {attribute: value}}.

test_masking_env.py starts it once per scenario, after running the real `map_masking_env`
from opt/shared.sh, so that every scenario loads the distro in a fresh interpreter, the way
a Lambda cold start does, and no cached configuration carries over between scenarios.

Each target drives a real app or client through the distro's own hooks:
- flask, fastapi, django: an app that receives `REQUEST` and answers with `RESPONSE`
- requests, httpx: a client that sends `REQUEST` to a local server that answers `RESPONSE`
- sqs: a boto3 client (stubbed with botocore's Stubber) that sends and receives messages
- redis, kafka, pika: the hooks the distro passes to the OpenTelemetry instrumentor
"""

import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Callable, Dict, List
from urllib.parse import urlencode

from opentelemetry import trace
from opentelemetry.sdk.trace import ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter

# Every payload carries the same three kinds of keys: `email` (personal data, not masked by
# default), `password` (masked by default) and `orderId` (never masked). Same as
# opt/node/test/masking-env.test.ts and the examples in docs/masking.md.
QUERY = {"email": "jane@example.com", "password": "q-pw", "orderId": "42"}
REQUEST_HEADERS = {
    "content-type": "application/json",
    "x-email": "jane@example.com",
    "x-password": "h-pw",
    "authorization": "Bearer abc",
    "x-signature": "sig",
    "x-orderid": "42",
}
REQUEST_BODY = {
    "email": "jane@example.com",
    "password": "b-pw",
    "apiToken": "t",
    "orderId": "42",
}
RESPONSE_HEADERS = {
    "x-email": "jane@example.com",
    "x-password": "h-pw",
    "x-orderid": "42",
}
RESPONSE_BODY = {
    "email": "jane@example.com",
    "password": "r-pw",
    "sessionToken": "s",
    "orderId": "42",
}
QUEUE_URL = "https://sqs.eu-central-1.amazonaws.com/123456789012/orders"
MESSAGE_ATTRIBUTES = {
    "email": {"DataType": "String", "StringValue": "jane@example.com"},
    "password": {"DataType": "String", "StringValue": "m-pw"},
    "orderId": {"DataType": "String", "StringValue": "42"},
}
MESSAGE_BODY = "order 42 for jane@example.com"
# A string payload that holds JSON, as sent to Redis, Kafka or RabbitMQ
JSON_TEXT = json.dumps(REQUEST_BODY)

PATH = "/orders?" + urlencode(QUERY)

exporter = InMemorySpanExporter()
provider = TracerProvider()
provider.add_span_processor(SimpleSpanProcessor(exporter))
trace.set_tracer_provider(provider)
tracer = trace.get_tracer("capture")


def install() -> None:
    from django.conf import settings

    settings.configure(
        ROOT_URLCONF=__name__, ALLOWED_HOSTS=["*"], MIDDLEWARE=[], USE_TZ=True
    )
    import django

    django.setup()

    from dash0_opentelemetry.instrumentations.botocore import instrumentor as botocore
    from dash0_opentelemetry.instrumentations.django import instrumentor as django_
    from dash0_opentelemetry.instrumentations.fastapi import instrumentor as fastapi
    from dash0_opentelemetry.instrumentations.flask import instrumentor as flask
    from dash0_opentelemetry.instrumentations.httpx import instrumentor as httpx
    from dash0_opentelemetry.instrumentations.requests import instrumentor as requests

    for instrumentor in (botocore, django_, fastapi, flask, httpx, requests):
        instrumentor.install_instrumentation()


def django_view(request: Any) -> Any:
    from django.http import JsonResponse

    return JsonResponse(RESPONSE_BODY, headers=RESPONSE_HEADERS)


def _urlpatterns() -> List[Any]:
    from django.urls import path

    return [path("orders", django_view)]


def __getattr__(name: str) -> Any:
    # Django reads `urlpatterns` from ROOT_URLCONF, this module, after setup.
    if name == "urlpatterns":
        return _urlpatterns()
    raise AttributeError(name)


def drive_flask() -> None:
    from flask import Flask, jsonify

    app = Flask("orders")

    @app.post("/orders")
    def orders() -> Any:
        return jsonify(RESPONSE_BODY), 200, RESPONSE_HEADERS

    app.test_client().post(PATH, data=json.dumps(REQUEST_BODY), headers=REQUEST_HEADERS)


def drive_fastapi() -> None:
    from fastapi import FastAPI, Request
    from fastapi.responses import JSONResponse
    from fastapi.testclient import TestClient

    app = FastAPI()

    @app.post("/orders")
    async def orders(request: Request) -> Any:
        await request.body()
        return JSONResponse(RESPONSE_BODY, headers=RESPONSE_HEADERS)

    TestClient(app).post(
        PATH, content=json.dumps(REQUEST_BODY), headers=REQUEST_HEADERS
    )


def drive_django() -> None:
    from django.test import Client

    Client().post(
        PATH,
        data=json.dumps(REQUEST_BODY),
        content_type="application/json",
        headers={k: v for k, v in REQUEST_HEADERS.items() if k != "content-type"},
    )


class _Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        self.rfile.read(int(self.headers.get("content-length", 0)))
        body = json.dumps(RESPONSE_BODY).encode()
        self.send_response(200)
        for name, value in {
            "content-type": "application/json",
            **RESPONSE_HEADERS,
        }.items():
            self.send_header(name, value)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args: Any) -> None:
        pass


def drive_http_clients() -> None:
    import httpx
    import requests

    server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_address[1]}/orders"
    try:
        with tracer.start_as_current_span("requests"):
            requests.post(url, data=json.dumps(REQUEST_BODY), headers=REQUEST_HEADERS)
        with tracer.start_as_current_span("httpx"):
            httpx.post(url, content=json.dumps(REQUEST_BODY), headers=REQUEST_HEADERS)
    finally:
        server.shutdown()


def drive_sqs() -> None:
    import boto3
    from botocore.stub import Stubber

    client = boto3.client(
        "sqs",
        region_name="eu-central-1",
        aws_access_key_id="testing",
        aws_secret_access_key="testing",  # pragma: allowlist secret
    )
    with Stubber(client) as stub:
        stub.add_response("send_message", {"MessageId": "m1"})
        client.send_message(
            QueueUrl=QUEUE_URL,
            MessageBody=MESSAGE_BODY,
            MessageAttributes=MESSAGE_ATTRIBUTES,
        )
        stub.add_response("send_message_batch", {"Successful": [], "Failed": []})
        client.send_message_batch(
            QueueUrl=QUEUE_URL,
            Entries=[
                {
                    "Id": "1",
                    "MessageBody": MESSAGE_BODY,
                    "MessageAttributes": MESSAGE_ATTRIBUTES,
                }
            ],
        )
        stub.add_response(
            "receive_message",
            {
                "Messages": [
                    {
                        "MessageId": "m1",
                        "Body": MESSAGE_BODY,
                        "MessageAttributes": MESSAGE_ATTRIBUTES,
                    }
                ]
            },
        )
        client.receive_message(QueueUrl=QUEUE_URL)


def _hooks_of(instrumentor_class: Any, install: Callable[[], None]) -> Dict[str, Any]:
    """Returns the hooks the distro passes to an OpenTelemetry instrumentor."""
    hooks: Dict[str, Any] = {}
    original = instrumentor_class.instrument
    instrumentor_class.instrument = lambda self, **kwargs: hooks.update(kwargs)
    try:
        install()
    finally:
        instrumentor_class.instrument = original
    return hooks


def drive_string_payloads() -> Dict[str, Dict[str, Any]]:
    from opentelemetry.instrumentation.kafka import KafkaInstrumentor
    from opentelemetry.instrumentation.pika import PikaInstrumentor
    from opentelemetry.instrumentation.redis import RedisInstrumentor

    from dash0_opentelemetry.instrumentations.kafka_python import instrumentor as kafka
    from dash0_opentelemetry.instrumentations.pika import instrumentor as pika
    from dash0_opentelemetry.instrumentations.redis import instrumentor as redis

    captured: Dict[str, Dict[str, Any]] = {}

    def capture(target: str, call: Callable[[Any], None]) -> None:
        span = tracer.start_span(target)
        call(span)
        span.end()
        captured.setdefault(target, {}).update(span.attributes or {})

    class Record:
        value = JSON_TEXT.encode()

    redis_hooks = _hooks_of(RedisInstrumentor, redis.install_instrumentation)
    capture(
        "redis",
        lambda s: redis_hooks["request_hook"](
            s, None, ["SET", "order:42", JSON_TEXT], {}
        ),
    )
    capture(
        "redis", lambda s: redis_hooks["response_hook"](s, None, JSON_TEXT.encode())
    )

    kafka_hooks = _hooks_of(KafkaInstrumentor, kafka.install_instrumentation)
    capture(
        "kafka",
        lambda s: kafka_hooks["produce_hook"](s, [], {"value": JSON_TEXT.encode()}),
    )
    capture("kafka", lambda s: kafka_hooks["consume_hook"](s, Record(), [], {}))

    pika_hooks = _hooks_of(PikaInstrumentor, pika.install_instrumentation)
    capture("pika", lambda s: pika_hooks["publish_hook"](s, JSON_TEXT.encode(), None))
    capture(
        "pika", lambda s: pika_hooks["consume_hook"](s, MESSAGE_BODY.encode(), None)
    )
    return captured


def _target_of(span: ReadableSpan) -> str:
    scope = span.instrumentation_scope.name if span.instrumentation_scope else ""
    for target in ("flask", "django", "requests", "httpx", "botocore"):
        if target in scope:
            return "sqs" if target == "botocore" else target
    if "fastapi" in scope or "asgi" in scope:
        return "fastapi"
    return scope


def main() -> None:
    install()
    drive_flask()
    drive_fastapi()
    drive_django()
    drive_http_clients()
    drive_sqs()

    captured: Dict[str, Dict[str, Any]] = {}
    for span in exporter.get_finished_spans():
        attributes = captured.setdefault(_target_of(span), {})
        for key, value in (span.attributes or {}).items():
            # SendMessage and SendMessageBatch both set messaging.publish.body
            name = key
            if (
                key == "messaging.publish.body"
                and span.attributes.get("rpc.method") == "SendMessageBatch"
            ):
                name = "messaging.publish.body#batch"
            # the first span wins: ASGI also records an `http.disconnect` receive
            attributes.setdefault(name, value)
    captured.update(drive_string_payloads())
    # Every span, for the check that no payload escapes masking through an attribute
    # the tests above don't name.
    captured["_spans"] = [  # type: ignore[assignment]
        {
            "target": _target_of(span),
            "name": span.name,
            "attributes": dict(span.attributes or {}),
        }
        for span in exporter.get_finished_spans()
    ]
    json.dump(captured, sys.stdout, default=str)


if __name__ == "__main__":
    main()

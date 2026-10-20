from typing import Any, Dict, List, Tuple

from opentelemetry.trace.span import Span

from dash0_opentelemetry.instrumentations import AbstractInstrumentor
from dash0_opentelemetry.libs.general_utils import dash0_safe_execute
from dash0_opentelemetry.libs.json_utils import dump_with_context
from dash0_opentelemetry.libs.masking import parse_query_string


def headers_from_environ(environ: Dict[str, Any]) -> Dict[str, str]:
    headers = {
        key[5:].replace("_", "-").lower(): value
        for key, value in environ.items()
        if key.startswith("HTTP_")
    }
    for key in ("CONTENT_TYPE", "CONTENT_LENGTH"):
        if environ.get(key):
            headers[key.replace("_", "-").lower()] = environ[key]
    return headers


class FlaskInstrumentorWrapper(AbstractInstrumentor):
    def __init__(self) -> None:
        super().__init__("flask")

    def assert_instrumented_package_importable(self) -> None:
        import flask  # noqa

    def install_instrumentation(self) -> None:
        from opentelemetry.instrumentation.flask import FlaskInstrumentor

        def request_hook(span: Span, flask_request_environ: Dict[str, Any]) -> None:
            with dash0_safe_execute("flask_request_hook"):
                span.set_attribute(
                    "http.request.headers",
                    dump_with_context(
                        "requestHeaders", headers_from_environ(flask_request_environ)
                    ),
                )
                query = flask_request_environ.get("QUERY_STRING")
                if query:
                    span.set_attribute(
                        "http.request.query",
                        dump_with_context("queryParams", parse_query_string(query)),
                    )

        def response_hook(
            span: Span, status: str, response_headers: List[Tuple[str, str]]
        ) -> None:
            with dash0_safe_execute("flask_response_hook"):
                span.set_attribute(
                    "http.response.headers",
                    dump_with_context("responseHeaders", dict(response_headers)),
                )

        FlaskInstrumentor().instrument(
            request_hook=request_hook, response_hook=response_hook
        )


instrumentor: AbstractInstrumentor = FlaskInstrumentorWrapper()

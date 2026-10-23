from opentelemetry.trace.span import Span

from dash0_opentelemetry.instrumentations import AbstractInstrumentor
from dash0_opentelemetry.instrumentations.instrumentation_utils import (
    add_body_attribute,
)
from dash0_opentelemetry.libs.general_utils import dash0_safe_execute
from dash0_opentelemetry.libs.json_utils import dump_with_context
from dash0_opentelemetry.libs.masking import parse_query_string


class DjangoInstrumentorWrapper(AbstractInstrumentor):
    def __init__(self) -> None:
        super().__init__("django")

    def assert_instrumented_package_importable(self) -> None:
        import django  # noqa

    def install_instrumentation(self) -> None:
        from django.http import HttpRequest, HttpResponse
        from opentelemetry.instrumentation.django import DjangoInstrumentor

        def request_hook(span: Span, request: HttpRequest) -> None:
            with dash0_safe_execute("django request_hook"):
                span.set_attribute(
                    "http.request.headers",
                    dump_with_context("requestHeaders", dict(request.headers)),
                )
                query = request.META.get("QUERY_STRING")
                if query:
                    span.set_attribute(
                        "http.request.query",
                        dump_with_context("queryParams", parse_query_string(query)),
                    )
                add_body_attribute(
                    span, request.body, "http.request.body", "requestBody"
                )

        def response_hook(
            span: Span, request: HttpRequest, response: HttpResponse
        ) -> None:
            with dash0_safe_execute("django response_hook"):
                span.set_attribute(
                    "http.response.headers",
                    dump_with_context("responseHeaders", dict(response.headers)),
                )
                add_body_attribute(
                    span, response.content, "http.response.body", "responseBody"
                )

        DjangoInstrumentor().instrument(
            request_hook=request_hook, response_hook=response_hook
        )


instrumentor: AbstractInstrumentor = DjangoInstrumentorWrapper()

from typing import Dict, Any

from lumigo_opentelemetry.libs.general_utils import lumigo_safe_execute
from opentelemetry.trace import Span, SpanKind

from lumigo_opentelemetry.instrumentations import AbstractInstrumentor
from lumigo_opentelemetry.instrumentations.botocore.parsers import AwsParser


class BotoCoreInstrumentorWrapper(AbstractInstrumentor):
    def __init__(self) -> None:
        super().__init__("botocore")

    def assert_instrumented_package_importable(self) -> None:
        from botocore.client import BaseClient  # noqa
        from botocore.endpoint import Endpoint  # noqa
        from botocore.exceptions import ClientError  # noqa

    def install_instrumentation(self) -> None:
        from opentelemetry.instrumentation.botocore import BotocoreInstrumentor
        from opentelemetry.instrumentation.boto3sqs import Boto3SQSInstrumentor

        BotocoreInstrumentor().instrument(
            request_hook=AwsParser.request_hook,
            response_hook=filtered_resource_hook,
        )
        Boto3SQSInstrumentor().instrument(
            request_hook=AwsParser.request_hook,
            response_hook=filtered_resource_hook,
        )


def filtered_resource_hook(
    span: Span, service_name: str, operation_name: str, result: Dict[Any, Any]
) -> None:
    AwsParser.response_hook(span, service_name, operation_name, result)


instrumentor: AbstractInstrumentor = BotoCoreInstrumentorWrapper()

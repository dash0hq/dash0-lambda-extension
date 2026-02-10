from dash0_opentelemetry.instrumentations import AbstractInstrumentor
from .common import SHOULD_INSTRUMENT_PAYLOADS


class GRPCInstrumentor(AbstractInstrumentor):
    def __init__(self) -> None:
        super().__init__("grpc")

    def assert_instrumented_package_importable(self) -> None:
        import grpc  # noqa

    @staticmethod
    def inject_interceptors() -> None:
        from .grpc_instrument_client import Dash0ClientInterceptor
        from .grpc_instrument_server import Dash0ServerInterceptor
        from opentelemetry.instrumentation.grpc import _client, _server

        _client.OpenTelemetryClientInterceptor = Dash0ClientInterceptor
        _server.OpenTelemetryServerInterceptor = Dash0ServerInterceptor

    def install_instrumentation(self) -> None:
        from opentelemetry.instrumentation.grpc import (
            GrpcInstrumentorServer,
            GrpcInstrumentorClient,
        )

        if SHOULD_INSTRUMENT_PAYLOADS:
            self.inject_interceptors()
        GrpcInstrumentorServer().instrument()
        GrpcInstrumentorClient().instrument()


instrumentor: AbstractInstrumentor = GRPCInstrumentor()

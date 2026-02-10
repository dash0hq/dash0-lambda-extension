from abc import abstractmethod, ABC
import os
from dash0_opentelemetry import logger
from dash0_opentelemetry.utils.config import get_disabled_instrumentations


class AbstractInstrumentor(ABC):
    """This class wraps around the facilities of opentelemetry.instrumentation.BaseInstrumentor
    to provide a safer baseline in terms of dependency checks than what is available upstream.
    """

    @abstractmethod
    def __init__(self, instrumentation_id: str):
        self._instrumentation_id = instrumentation_id

    def is_applicable(self) -> bool:
        # Check if this instrumentation is explicitly disabled
        disabled_instrumentations = get_disabled_instrumentations()
        if self.instrumentation_id in disabled_instrumentations:
            logger.info(
                "Instrumentation '%s' is disabled via DASH0_DISABLE_INSTRUMENTATION",
                self.instrumentation_id,
            )
            return False

        try:
            self.assert_instrumented_package_importable()
            return True
        except ImportError:
            return False

    @abstractmethod
    def assert_instrumented_package_importable(self) -> None:
        raise Exception(
            "'assert_instrumented_package_importable' method not implemented!"
        )

    @abstractmethod
    def install_instrumentation(self) -> None:
        raise Exception("'apply_instrumentation' method not implemented!")

    @property
    def instrumentation_id(self) -> str:
        return self._instrumentation_id

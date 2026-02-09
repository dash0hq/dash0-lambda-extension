from __future__ import annotations

import logging
import os
import sys
from typing import Any, Dict, TypeVar


LOG_FORMAT = "#LUMIGO# - %(asctime)s - %(levelname)s - %(message)s"
DEFAULT_TIMEOUT_MS = 1000
MAX_FLUSH_TIMEOUT_MS = 10000  # 10 seconds
USING_DEFAULT_TIMEOUT_MESSAGE = f"Using default {DEFAULT_TIMEOUT_MS}ms timeout."

T = TypeVar("T")


def _setup_logger(logger_name: str = "lumigo-opentelemetry") -> logging.Logger:
    """
    This function returns Lumigo's logger. The Lumigo logger prints to stderr.
    The Lumigo logger is set to INFO by default. If the environment variable
    `LUMIGO_DEBUG=true` is set, the severity is set to DEBUG.
    """
    _logger = logging.getLogger(logger_name)
    _logger.propagate = False
    handler = logging.StreamHandler()
    handler.setFormatter(logging.Formatter(LOG_FORMAT))
    if os.environ.get("LUMIGO_DEBUG", "").lower() == "true":
        _logger.setLevel(logging.DEBUG)
    else:
        _logger.setLevel(logging.INFO)
    _logger.addHandler(handler)

    # Suppress spurious warnings when the application is not running on ECS
    logging.getLogger("opentelemetry.sdk.extension.aws.resource.ecs").setLevel(
        logging.CRITICAL
    )

    # Suppress spurious warnings when the application is not running on EKS
    logging.getLogger("opentelemetry.sdk.extension.aws.resource.eks").setLevel(
        logging.CRITICAL
    )

    return _logger


logger = _setup_logger()


def auto_load(_: Any) -> None:
    """
    Called when injection performed over `AUTOWRAPT_BOOTSTRAP`.
    """
    # Some versions of Python have issues with the 'argv' attribute when
    # auto-loading the tracer. See https://bugs.python.org/issue32573
    import sys

    if not hasattr(sys, "argv"):
        sys.argv = [""]

    # We do not need to init the package, it will happen automatically due
    # to the init() call at the end of this file.


def init() -> Dict[str, Any]:
    """Initialize the Lumigo OpenTelemetry distribution."""

    try:
        python_version = sys.version_info
        # Check if the major version is 3 and the minor version is between 9 and 14
        if python_version.major != 3 or not (9 <= python_version.minor <= 14):
            logger.warning(
                f"Unsupported Python version {python_version.major}.{python_version.minor}; "
                "only Python 3.9 to 3.14 are supported."
            )
            return {}

    except Exception as e:  # Catch any issues with accessing sys.version_info
        # Log a warning if there is a failure in verifying the Python version
        logger.warning("Failed to verify the Python version due to: %s", str(e))
        return {}

    if str(os.environ.get("LUMIGO_SWITCH_OFF", False)).lower() == "true":
        logger.info(
            "Lumigo OpenTelemetry distribution disabled via the 'LUMIGO_SWITCH_OFF' environment variable"
        )
        return {}

    # Multiple packages are passed to autowrapt in comma-separated form
    if "lumigo_opentelemetry" in os.getenv("AUTOWRAPT_BOOTSTRAP", "").split(","):
        activation_mode = "automatic injection"
    else:
        activation_mode = "import"

    logger.info(
        f"Loading the Dash0 OpenTelemetry distribution (injection mode: %s)",
        activation_mode,
    )

    from opentelemetry import trace
    from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
    from opentelemetry.sdk.trace.export import BatchSpanProcessor
    from opentelemetry.sdk.trace import SpanLimits, TracerProvider

    LUMIGO_ENDPOINT_BASE_URL = "https://ga-otlp.lumigo-tracer-edge.golumigo.com/v1"

    DEFAULT_LUMIGO_ENDPOINT = f"{LUMIGO_ENDPOINT_BASE_URL}/traces"

    lumigo_traces_endpoint = os.getenv("LUMIGO_ENDPOINT", DEFAULT_LUMIGO_ENDPOINT)
    lumigo_token = os.getenv("LUMIGO_TRACER_TOKEN")
    tracing_enabled = os.getenv("LUMIGO_ENABLE_TRACES", "true").lower() == "true"
    spandump_file = os.getenv("LUMIGO_DEBUG_SPANDUMP")

    # Activate instrumentations
    from lumigo_opentelemetry.instrumentations import instrumentations  # noqa
    from lumigo_opentelemetry.instrumentations.instrumentations import framework
    from lumigo_opentelemetry.libs.general_utils import get_max_size
    from lumigo_opentelemetry.resources.detectors import (
        get_process_resource,
        get_resource,
    )

    process_resource = get_process_resource()

    resource = get_resource(
        process_resource, {"framework": framework}
    )

    tracer_provider = TracerProvider(
        resource=resource,
        span_limits=(SpanLimits(max_span_attribute_length=(get_max_size()))),
    )

    if lumigo_token:
        if tracing_enabled:
            tracer_provider.add_span_processor(
                BatchSpanProcessor(
                    OTLPSpanExporter(
                        endpoint=lumigo_traces_endpoint,
                        headers={"Authorization": f"LumigoToken {lumigo_token}"},
                    ),
                )
            )
        else:
            logger.info(
                'Tracing is disabled (the "LUMIGO_ENABLE_TRACES" environment variable is not set to "true"): no traces will be sent to Lumigo.'
            )
    else:
        logger.warning(
            "Lumigo token not provided (env var 'LUMIGO_TRACER_TOKEN' not set); "
            "no data will be sent to Lumigo"
        )

    if spandump_file:
        from opentelemetry.sdk.trace.export import (
            ConsoleSpanExporter,
            SimpleSpanProcessor,
        )

        tracer_provider.add_span_processor(
            SimpleSpanProcessor(
                ConsoleSpanExporter(
                    out=open(spandump_file, "w"),
                    # Print one span per line for ease of parsing, as the
                    # file itself will not be valid JSON, it will be just a
                    # sequence of JSON objects, not a list
                    formatter=lambda span: span.to_json(indent=None) + "\n",
                )
            )
        )

        logger.debug("Storing a copy of the trace data under: %s", spandump_file)

    trace.set_tracer_provider(tracer_provider)

    return {"tracer_provider": tracer_provider}


# Load the package on import
init_data = init()

tracer_provider = init_data.get("tracer_provider")

__all__ = [
    "auto_load",
    "init",
    "logger",
    "tracer_provider",
]

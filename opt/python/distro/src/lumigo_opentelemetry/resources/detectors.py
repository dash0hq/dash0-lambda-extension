import logging
import os
from typing import Any, Dict

from opentelemetry.sdk.resources import (
    ResourceDetector,
    Resource,
    get_aggregated_resources,
)

import lumigo_opentelemetry
from lumigo_opentelemetry.libs.json_utils import dump_with_context

logger = logging.getLogger(__name__)


ENV_ATTR_NAME = "process.environ"


class EnvVarsDetector(ResourceDetector):
    def detect(self) -> "Resource":
        return Resource(
            {ENV_ATTR_NAME: dump_with_context("environment", dict(os.environ))}
        )


def get_process_resource() -> "Resource":
    return get_aggregated_resources(
        detectors=[
            EnvVarsDetector(),
        ],
    )


def get_resource(
    process_resource: "Resource",
    attributes: Dict[str, Any],
) -> "Resource":
    return (
        Resource.create(attributes=attributes)
        .merge(process_resource)
    )

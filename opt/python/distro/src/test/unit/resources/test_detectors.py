import json
import os
import re

from opentelemetry.sdk import resources
from opentelemetry.semconv.resource import ResourceAttributes
from lumigo_core.configuration import CoreConfiguration

from dash0_opentelemetry.resources.detectors import (
    EnvVarsDetector,
    get_resource,
    get_process_resource,
)

from dash0_opentelemetry import _setup_logger


def test_env_vars_detector(monkeypatch):
    for key in os.environ:
        monkeypatch.delenv(key)
    monkeypatch.setenv("a", "b")
    monkeypatch.setenv("k", "v")
    monkeypatch.setenv("secret", "value")

    resource = EnvVarsDetector().detect()

    assert resource.attributes["process.environ"] == json.dumps(
        {"a": "b", "k": "v", "secret": "****"}
    )


def test_env_vars_detector_specific_config(monkeypatch):
    monkeypatch.setattr(
        CoreConfiguration, "secret_masking_regex_environment", re.compile("specific.*")
    )
    for key in os.environ:
        monkeypatch.delenv(key)
    monkeypatch.setenv("a", "b")
    monkeypatch.setenv("c", "d")
    monkeypatch.setenv("specific_env_var", "value")

    resource = EnvVarsDetector().detect()

    assert resource.attributes["process.environ"] == json.dumps(
        {"a": "b", "c": "d", "specific_env_var": "****"}
    )

"""
DASH0_MASK_IN_EXTENSION hands payload masking from the distro to the extension.

Runs the real `map_masking_env` from opt/shared.sh, loads the resulting environment into
lumigo_core and dumps a payload through each context the instrumentations use.
"""

import json
import os
import subprocess
from pathlib import Path
from typing import Any

import pytest
from lumigo_core.configuration import CoreConfiguration
from lumigo_core.scrubbing import get_omitting_regex

from dash0_opentelemetry.libs.json_utils import dump, dump_with_context

SHARED_SH = Path(__file__).resolve().parents[6] / "shared.sh"
CONTEXTS = ["requestBody", "requestHeaders", "responseBody", "responseHeaders"]
PAYLOAD = {"password": "XQZSECRET", "email": "jane@example.com"}


def masking_env(**env: str) -> dict:
    out = subprocess.run(
        ["bash", "-c", 'source "$1" && map_masking_env && env -0', "bash", str(SHARED_SH)],
        env={"PATH": os.environ["PATH"], **env},
        capture_output=True,
        check=True,
    ).stdout.decode()
    entries = (e.split("=", 1) for e in out.split("\0") if e)
    return {k: v for k, v in entries if k.startswith("LUMIGO_")}


@pytest.fixture
def load_env(monkeypatch):
    def load(**env: str) -> None:
        for key in list(os.environ):
            if key.startswith(("LUMIGO_", "DASH0_MASK_")):
                monkeypatch.delenv(key)
        for key, value in masking_env(**env).items():
            monkeypatch.setenv(key, value)
        for attr in vars(CoreConfiguration):
            if attr.startswith("secret_masking_regex_"):
                monkeypatch.setattr(CoreConfiguration, attr, None)
        CoreConfiguration.load_from_env_vars()
        get_omitting_regex.cache_clear()

    yield load
    get_omitting_regex.cache_clear()


def captured() -> list:
    return [json.loads(dump_with_context(c, PAYLOAD)) for c in CONTEXTS] + [
        json.loads(dump(PAYLOAD))
    ]


def test_flag_unset_distro_masks_with_defaults(load_env):
    load_env()
    assert all(c["password"] == "****" for c in captured())


@pytest.mark.parametrize("flag", ["true", "1", "yes"])
def test_flag_on_distro_masks_nothing(load_env, flag):
    load_env(DASH0_MASK_IN_EXTENSION=flag)
    assert all(c == PAYLOAD for c in captured())


def test_flag_on_ignores_mask_rules_and_lumigo_vars(load_env):
    load_env(
        DASH0_MASK_IN_EXTENSION="true",
        DASH0_MASK_RULES='[".*email.*"]',
        DASH0_MASK_REQUEST_BODY="all",
        LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES="all",
    )
    assert all(c == PAYLOAD for c in captured())


def test_flag_on_still_truncates(load_env, monkeypatch):
    load_env(DASH0_MASK_IN_EXTENSION="true")
    monkeypatch.setenv("OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT", "10")
    assert dump_with_context("requestBody", PAYLOAD).endswith("...[too long]")


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: dict = {}

    def set_attribute(self, key: str, value: Any) -> None:
        self.attributes[key] = value

    def set_attributes(self, attributes: dict) -> None:
        self.attributes.update(attributes)


def test_flag_on_captures_what_distro_captured_python_json_holds(load_env):
    """tests/masking_e2e.rs sends these exact captures through the extension and checks it
    masks them. Regenerate with UPDATE_FIXTURES=1 when the distro's capture changes."""
    from dash0_opentelemetry.instrumentations.botocore.parsers import AwsParser

    load_env(DASH0_MASK_IN_EXTENSION="true")
    secret = "XQZ-E2E-SECRET"
    span = RecordingSpan()
    AwsParser.request_hook(
        span,
        "dynamodb",
        "PutItem",
        {"TableName": "orders", "Item": {"email": "jane", "password": secret}},
    )
    AwsParser.response_hook(
        span,
        "dynamodb",
        "GetItem",
        {
            "Item": {"email": "jane", "apiToken": secret},
            "ResponseMetadata": {
                "HTTPStatusCode": 200,
                "HTTPHeaders": {"x-email": "jane", "x-api-key": secret},
            },
        },
    )
    captured = {k: v for k, v in span.attributes.items() if secret in str(v)}

    file = SHARED_SH.parents[1] / "tests" / "testdata" / "distro_captured_python.json"
    if os.environ.get("UPDATE_FIXTURES"):
        file.write_text(json.dumps(captured, indent=2) + "\n")
    assert captured == json.loads(file.read_text())
    assert set(captured) == {
        "http.request.body",
        "http.response.body",
        "http.response.headers",
    }

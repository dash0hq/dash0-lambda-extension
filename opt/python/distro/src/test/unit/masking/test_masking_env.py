"""
SIG-700: the documented DASH0_MASK_* env vars must control masking of the payload
attributes the Python distro captures (docs/masking.md).

`opt/shared.sh` maps DASH0_MASK_* onto LUMIGO_SECRET_MASKING_REGEX* in the exec wrapper,
before the runtime starts; the distro reads those (dash0_opentelemetry/libs/masking.py). The
mapping itself is tested in opt/test/shared.test.sh. Each scenario here runs the real
`map_masking_env` in bash, then capture_payloads.py in a fresh interpreter with the resulting
environment, which drives real apps and clients through the distro's hooks. A fresh process
per scenario means no configuration cached by one scenario can satisfy another.

Same scenarios as opt/node/test/masking-env.test.ts.
"""

import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from functools import lru_cache
from pathlib import Path
from typing import Any, Dict, FrozenSet, List, Tuple

import pytest

from dash0_opentelemetry.libs.masking import DEFAULT_MASKING_PATTERNS

DISTRO_ROOT = Path(__file__).resolve().parents[4]
REPO_ROOT = DISTRO_ROOT.parents[2]
SHARED_SH = REPO_ROOT / "opt" / "shared.sh"
CAPTURE = Path(__file__).with_name("capture_payloads.py")

MASKED = "****"
MASKING_ENV = ("DASH0_MASK_", "LUMIGO_SECRET_MASKING_REGEX", "LUMIGO_BLACKLIST_REGEX")

QUERY = "http.request.query"
REQUEST_HEADERS = "http.request.headers"
REQUEST_BODY = "http.request.body"
RESPONSE_HEADERS = "http.response.headers"
RESPONSE_BODY = "http.response.body"
PUBLISH_BODY = "messaging.publish.body"
PUBLISH_BATCH_BODY = "messaging.publish.body#batch"
CONSUME_BODY = "messaging.consume.body"

# What each target captures. Flask captures no bodies; httpx no response body.
CAPTURED: Dict[str, List[str]] = {
    "flask": [QUERY, REQUEST_HEADERS, RESPONSE_HEADERS],
    "fastapi": [QUERY, REQUEST_HEADERS, REQUEST_BODY, RESPONSE_HEADERS, RESPONSE_BODY],
    "django": [QUERY, REQUEST_HEADERS, REQUEST_BODY, RESPONSE_HEADERS, RESPONSE_BODY],
    "requests": [REQUEST_HEADERS, REQUEST_BODY, RESPONSE_HEADERS, RESPONSE_BODY],
    "httpx": [REQUEST_HEADERS, REQUEST_BODY, RESPONSE_HEADERS],
    "sqs": [PUBLISH_BODY, PUBLISH_BATCH_BODY, CONSUME_BODY],
}
ALL: List[Tuple[str, str]] = [(t, a) for t, attrs in CAPTURED.items() for a in attrs]

# The variable that controls each attribute, as documented in docs/masking.md
SCOPES = {
    "DASH0_MASK_QUERY_PARAMS": [QUERY],
    "DASH0_MASK_REQUEST_HEADERS": [REQUEST_HEADERS],
    "DASH0_MASK_REQUEST_BODY": [REQUEST_BODY, PUBLISH_BODY, PUBLISH_BATCH_BODY],
    "DASH0_MASK_RESPONSE_HEADERS": [RESPONSE_HEADERS],
    "DASH0_MASK_RESPONSE_BODY": [RESPONSE_BODY, CONSUME_BODY],
}


def scoped(*attributes: str) -> List[Tuple[str, str]]:
    return [(t, a) for t, a in ALL if a in attributes]


def except_(*attributes: str) -> List[Tuple[str, str]]:
    return [(t, a) for t, a in ALL if a not in attributes]


def map_masking_env(env: Dict[str, str]) -> Tuple[Dict[str, str], str]:
    """Runs `map_masking_env` from opt/shared.sh; returns the masking env and stderr."""
    result = subprocess.run(
        ["bash", "-c", 'source "$1" && map_masking_env && env -0', "bash", SHARED_SH],
        env={"PATH": os.environ["PATH"], **env},
        capture_output=True,
        check=True,
    )
    mapped = dict(
        entry.split("=", 1)
        for entry in result.stdout.decode().split("\0")
        if entry.startswith(MASKING_ENV)
    )
    return mapped, result.stderr.decode()


@lru_cache(maxsize=None)
def _capture(env: FrozenSet[Tuple[str, str]]) -> Dict[str, Any]:
    mapped, wrapper_stderr = map_masking_env(dict(env))
    process_env = {k: v for k, v in os.environ.items() if not k.startswith(MASKING_ENV)}
    process_env.update(mapped)
    process_env.update(
        {
            "PYTHONPATH": str(DISTRO_ROOT / "src"),
            "DASH0_SWITCH_OFF": "true",
            "DASH0_DISABLE_PYTHON_DEPENDENCY_CHECK": "true",
        }
    )
    result = subprocess.run(
        [sys.executable, str(CAPTURE)],
        env=process_env,
        cwd=DISTRO_ROOT,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr.decode()
    return {
        "attributes": json.loads(result.stdout),
        "stderr": wrapper_stderr + result.stderr.decode(),
    }


def capture(**env: str) -> Dict[str, Dict[str, str]]:
    return _capture(frozenset(env.items()))["attributes"]  # type: ignore


def stderr_of(**env: str) -> str:
    return _capture(frozenset(env.items()))["stderr"]  # type: ignore


def values_of(value: str, suffix: str) -> List[Any]:
    """Values of every key in the captured JSON whose name ends with `suffix`, any case."""
    found: List[Any] = []

    def walk(node: Any) -> None:
        if isinstance(node, list):
            for item in node:
                walk(item)
        elif isinstance(node, dict):
            for key, child in node.items():
                if key.lower().endswith(suffix.lower()):
                    found.append(child)
                else:
                    walk(child)

    walk(json.loads(value))
    return found


def assert_masked(
    captured: Dict[str, Dict[str, str]], where: List[Tuple[str, str]], key: str
) -> None:
    for target, attribute in where:
        values = values_of(captured[target][attribute], key)
        assert values and all(v == MASKED for v in values), (
            target,
            attribute,
            key,
            values,
        )


def assert_clear(
    captured: Dict[str, Dict[str, str]], where: List[Tuple[str, str]], key: str
) -> None:
    for target, attribute in where:
        values = values_of(captured[target][attribute], key)
        assert values and MASKED not in values, (target, attribute, key, values)


DEFAULT_RULES = json.dumps(DEFAULT_MASKING_PATTERNS)
EMAIL = '[".*email.*"]'
ORDER_ID = '[".*orderId.*"]'

SCENARIOS: List[Dict[str, str]] = [
    {},
    {"DASH0_MASK_RULES": EMAIL},
    {"DASH0_MASK_RULES": "email"},
    {"DASH0_MASK_RULES": '["email"]'},
    {"DASH0_MASK_RULES": '["^email$"]'},
    {"DASH0_MASK_RULES": json.dumps(DEFAULT_MASKING_PATTERNS + [".*email.*"])},
    {"DASH0_MASK_RULES": '[".*MessageBody.*","^Body$"]'},
    {"DASH0_MASK_RULES": "not json"},
    {"DASH0_MASK_RULES": '[".*email.*",]'},
    {"DASH0_MASK_RULES": '["(", ".*email.*"]'},
    {"DASH0_MASK_RULES": "all"},
    {"DASH0_MASK_RULES": EMAIL, "DASH0_MASK_REQUEST_BODY": "nope"},
    *({var: EMAIL} for var in SCOPES),
    *({"DASH0_MASK_RULES": ORDER_ID, var: EMAIL} for var in SCOPES),
    *({var: "all"} for var in SCOPES),
    {"LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES": EMAIL},
    {"LUMIGO_BLACKLIST_REGEX": EMAIL},
    {"LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES": '["("'},
    {
        "LUMIGO_SECRET_MASKING_REGEX": ORDER_ID,
        "DASH0_MASK_RULES": EMAIL,
        "LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES": ORDER_ID,
        "DASH0_MASK_REQUEST_BODY": EMAIL,
    },
]


@pytest.fixture(scope="module", autouse=True)
def run_scenarios_in_parallel() -> None:
    with ThreadPoolExecutor(max_workers=8) as pool:
        list(pool.map(lambda env: _capture(frozenset(env.items())), SCENARIOS))


class TestDefaults:
    def test_mask_passwords_in_every_captured_attribute(self) -> None:
        assert_masked(capture(), ALL, "password")

    def test_mask_tokens_authorization_and_signature(self) -> None:
        captured = capture()
        assert_masked(captured, scoped(REQUEST_BODY), "apiToken")
        assert_masked(captured, scoped(RESPONSE_BODY), "sessionToken")
        assert_masked(captured, scoped(REQUEST_HEADERS), "authorization")
        assert_masked(captured, scoped(REQUEST_HEADERS), "x-signature")

    def test_leave_email_and_order_id_in_clear_text(self) -> None:
        captured = capture()
        assert_clear(captured, ALL, "email")
        assert_clear(captured, ALL, "orderId")

    def test_are_the_unified_defaults_of_the_extension_and_the_wrapper(self) -> None:
        shared_sh = SHARED_SH.read_text()
        line = next(
            l
            for l in shared_sh.splitlines()
            if l.startswith("DASH0_DEFAULT_MASK_RULES=")
        )
        assert json.loads(line.split("=", 1)[1].strip("'")) == DEFAULT_MASKING_PATTERNS
        masking_rs = (REPO_ROOT / "src" / "otlp" / "masking.rs").read_text()
        for pattern in DEFAULT_MASKING_PATTERNS:
            assert f'"{pattern}"' in masking_rs


class TestRules:
    def test_masks_matching_keys_in_every_captured_attribute(self) -> None:
        assert_masked(capture(DASH0_MASK_RULES=EMAIL), ALL, "email")

    def test_replaces_the_default_rules(self) -> None:
        assert_clear(capture(DASH0_MASK_RULES=EMAIL), ALL, "password")

    def test_can_extend_the_defaults_by_listing_them(self) -> None:
        captured = capture(
            DASH0_MASK_RULES=json.dumps(DEFAULT_MASKING_PATTERNS + [".*email.*"])
        )
        assert_masked(captured, ALL, "email")
        assert_masked(captured, ALL, "password")
        assert_clear(captured, ALL, "orderId")

    def test_patterns_match_anywhere_in_the_key(self) -> None:
        # `email` masks `x-email`, as in the extension and the Node distro
        assert_masked(capture(DASH0_MASK_RULES='["email"]'), ALL, "email")

    def test_anchored_patterns_match_the_exact_key_only(self) -> None:
        captured = capture(DASH0_MASK_RULES='["^email$"]')
        assert_masked(captured, except_(REQUEST_HEADERS, RESPONSE_HEADERS), "email")
        assert_clear(captured, scoped(REQUEST_HEADERS, RESPONSE_HEADERS), "x-email")

    def test_masks_the_whole_sqs_message_body_when_it_names_the_key(self) -> None:
        sqs = capture(DASH0_MASK_RULES='[".*MessageBody.*","^Body$"]')["sqs"]
        assert json.loads(sqs[PUBLISH_BODY])["MessageBody"] == MASKED
        assert (
            json.loads(sqs[PUBLISH_BATCH_BODY])["Entries"][0]["MessageBody"] == MASKED
        )
        assert json.loads(sqs[CONSUME_BODY])["Messages"][0]["Body"] == MASKED

    @pytest.mark.parametrize("value", ["not json", "email", '[".*email.*",]'])
    def test_falls_back_to_the_defaults_when_it_is_invalid(self, value: str) -> None:
        captured = capture(DASH0_MASK_RULES=value)
        assert_masked(captured, ALL, "password")
        assert_clear(captured, ALL, "email")
        assert "[dash0] Ignoring DASH0_MASK_RULES" in stderr_of(DASH0_MASK_RULES=value)

    def test_drops_an_invalid_pattern_and_keeps_the_others(self) -> None:
        env = {"DASH0_MASK_RULES": '["(", ".*email.*"]'}
        assert_masked(capture(**env), ALL, "email")
        assert "shutting down" not in stderr_of(**env)

    def test_still_applies_to_a_payload_whose_own_var_is_invalid(self) -> None:
        captured = capture(DASH0_MASK_RULES=EMAIL, DASH0_MASK_REQUEST_BODY="nope")
        assert_masked(captured, scoped(*SCOPES["DASH0_MASK_REQUEST_BODY"]), "email")

    def test_all_replaces_every_payload(self) -> None:
        captured = capture(DASH0_MASK_RULES="all")
        for target, attribute in ALL:
            assert captured[target][attribute] == json.dumps(MASKED), (
                target,
                attribute,
            )


@pytest.mark.parametrize("env_var", list(SCOPES))
class TestPerPayloadVariables:
    def test_masks_matching_keys_in_its_own_attributes_only(self, env_var: str) -> None:
        captured = capture(**{env_var: EMAIL})
        assert_masked(captured, scoped(*SCOPES[env_var]), "email")
        assert_clear(captured, except_(*SCOPES[env_var]), "email")

    def test_keeps_the_defaults_for_every_other_attribute(self, env_var: str) -> None:
        captured = capture(**{env_var: EMAIL})
        assert_masked(captured, except_(*SCOPES[env_var]), "password")
        assert_clear(captured, scoped(*SCOPES[env_var]), "password")

    def test_takes_precedence_over_the_rules_for_its_own_attributes(
        self, env_var: str
    ) -> None:
        captured = capture(**{"DASH0_MASK_RULES": ORDER_ID, env_var: EMAIL})
        assert_masked(captured, scoped(*SCOPES[env_var]), "email")
        assert_clear(captured, scoped(*SCOPES[env_var]), "orderId")
        assert_masked(captured, except_(*SCOPES[env_var]), "orderId")

    def test_all_replaces_its_own_attributes_only(self, env_var: str) -> None:
        captured = capture(**{env_var: "all"})
        for target, attribute in scoped(*SCOPES[env_var]):
            assert captured[target][attribute] == json.dumps(MASKED), (
                target,
                attribute,
            )
        assert_masked(captured, except_(*SCOPES[env_var]), "password")
        assert_clear(captured, except_(*SCOPES[env_var]), "email")


class TestLegacyLumigoVariables:
    def test_still_work_on_their_own(self) -> None:
        captured = capture(LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES=EMAIL)
        assert_masked(captured, scoped(*SCOPES["DASH0_MASK_REQUEST_BODY"]), "email")

    def test_blacklist_regex_still_works_on_its_own(self) -> None:
        assert_masked(capture(LUMIGO_BLACKLIST_REGEX=EMAIL), ALL, "email")

    def test_are_overridden_by_the_dash0_equivalent(self) -> None:
        captured = capture(
            LUMIGO_SECRET_MASKING_REGEX=ORDER_ID,
            DASH0_MASK_RULES=EMAIL,
            LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES=ORDER_ID,
            DASH0_MASK_REQUEST_BODY=EMAIL,
        )
        assert_masked(captured, ALL, "email")
        assert_clear(captured, ALL, "orderId")

    def test_an_unparsable_value_falls_back_without_a_misleading_critical_log(
        self,
    ) -> None:
        env = {"LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES": '["("'}
        assert_masked(capture(**env), ALL, "password")
        assert "shutting down" not in stderr_of(**env)
        assert "Ignoring LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES" in stderr_of(
            **env
        )


class TestStringPayloads:
    """Redis commands and replies, and Kafka and RabbitMQ messages, are captured as text."""

    def test_text_that_is_not_json_is_kept_as_it_is(self) -> None:
        captured = capture(DASH0_MASK_RULES=EMAIL)
        assert captured["redis"]["db.statement"].startswith("SET order:42 ")
        assert "jane@example.com" in captured["redis"]["db.statement"]
        assert captured["pika"][CONSUME_BODY] == "order 42 for jane@example.com"

    def test_text_that_holds_json_is_masked_by_key(self) -> None:
        captured = capture(DASH0_MASK_RULES=EMAIL)
        for target, attribute in [
            ("redis", "db.response.body"),
            ("kafka", "messaging.produce.body"),
            ("kafka", CONSUME_BODY),
            ("pika", PUBLISH_BODY),
        ]:
            assert json.loads(captured[target][attribute])["email"] == MASKED

    @pytest.mark.parametrize(
        "env_var,attributes",
        [
            ("DASH0_MASK_REQUEST_BODY", [("redis", "db.statement")]),
            (
                "DASH0_MASK_RESPONSE_BODY",
                [
                    ("redis", "db.response.body"),
                    ("kafka", CONSUME_BODY),
                    ("pika", CONSUME_BODY),
                ],
            ),
            # Like the Node distro, sent messages follow the query-parameter rules
            (
                "DASH0_MASK_QUERY_PARAMS",
                [("kafka", "messaging.produce.body"), ("pika", PUBLISH_BODY)],
            ),
        ],
    )
    def test_all_replaces_the_text(
        self, env_var: str, attributes: List[Tuple[str, str]]
    ) -> None:
        captured = capture(**{env_var: "all"})
        for target, attribute in attributes:
            assert captured[target][attribute] == json.dumps(MASKED), (
                target,
                attribute,
            )

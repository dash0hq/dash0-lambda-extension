"""
Known masking gaps: payloads the Python distro exports with a secret in clear text, even
with the default masking patterns.

Found by the cross-runtime masking audit:
https://claude.ai/code/artifact/4b7225a4-2bd2-424b-8034-b4d57da3e112

Convention:
- Each case runs one scenario of capture_gaps.py in a fresh interpreter, with the
  environment the real `map_masking_env` from opt/shared.sh produces. The scenario drives a
  real library through the distro's hooks or the upstream instrumentation, and puts
  `s3cr3t-<gap id>` into the payload, under a key the defaults match (password, cookie, ...)
  or under the key the gap is about (SSM Value, KMS Plaintext, ...).
- Each case asserts the outcome we want: the secret is not in the named attributes.
- A case for an open gap is marked `xfail(strict=True)` with its stable id GAP-PY-<n>.
  Once someone fixes the gap, the case passes, strict xfail turns it red, and the reason
  says "gap fixed: promote this test". Then drop the mark and keep it as a regression test.
- Cases without the mark were audit findings that didn't reproduce. They stay as
  regression tests.

Not testable here, so not listed: langchain prompts and completions (GAP-PY-9; the pinned
opentelemetry-instrumentation-langchain 0.50.1 fails to install with wrapt 2), agno
messages (GAP-PY-10; no offline model), and psycopg/pymysql tuple results (they need a
database server).
"""

import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from functools import lru_cache
from pathlib import Path
from typing import Any, Dict, List, NamedTuple, Tuple

import pytest

from test_masking_env import DISTRO_ROOT, MASKING_ENV, map_masking_env

CAPTURE = Path(__file__).with_name("capture_gaps.py")

HTTP_URL = ("http.url", "http.target", "url.full", "url.query")


class Case(NamedTuple):
    gap: str
    scenario: str
    # The attributes that must not hold the secret, as names or name prefixes
    attributes: Tuple[str, ...]
    # Extra environment, such as an OpenTelemetry opt-in
    env: Tuple[Tuple[str, str], ...] = ()
    events: bool = False


def gap(reason: str, *cases: Case) -> List[Any]:
    return [
        pytest.param(
            case,
            id=f"{case.gap}-{case.scenario}",
            marks=pytest.mark.xfail(
                strict=True,
                raises=AssertionError,
                reason=f"{case.gap}: {reason} (if this passes, the gap is fixed: "
                "promote this test)",
            ),
        )
        for case in cases
    ]


def not_reproduced(*cases: Case) -> List[Any]:
    return [pytest.param(case, id=f"{case.gap}-{case.scenario}") for case in cases]


CASES = [
    *gap(
        "the query string stays in the URL attributes of server spans",
        Case("GAP-PY-1", "lambda_api_gateway_query", HTTP_URL),
        Case("GAP-PY-1", "flask_query", HTTP_URL),
        Case("GAP-PY-1", "fastapi_query", HTTP_URL),
        Case("GAP-PY-1", "django_query", HTTP_URL),
    ),
    *gap(
        "the query string stays in http.url of client spans",
        Case("GAP-PY-2", "requests_query", HTTP_URL),
        Case("GAP-PY-2", "httpx_query", HTTP_URL),
    ),
    *gap(
        "form, multipart and text bodies aren't JSON, so they are kept as they are",
        Case("GAP-PY-3", "requests_form_body", ("http.request.body",)),
        Case("GAP-PY-3", "requests_multipart_body", ("http.request.body",)),
        Case("GAP-PY-3", "httpx_form_body", ("http.request.body",)),
        Case("GAP-PY-3", "httpx_text_body", ("http.request.body",)),
        Case("GAP-PY-3", "django_form_body", ("http.request.body",)),
    ),
    *gap(
        "AWS SDK secrets under keys the defaults don't match",
        Case("GAP-PY-4", "ssm_parameter_value", ("http.response.body",)),
        Case("GAP-PY-4", "kms_plaintext", ("http.response.body",)),
        Case("GAP-PY-4", "dynamodb_expression_values", ("http.request.body",)),
    ),
    *gap(
        "JSON inside a string AWS SDK parameter isn't parsed, so its keys aren't masked",
        Case("GAP-PY-5", "sqs_message_body_json", ("messaging.publish.body",)),
        Case("GAP-PY-5", "eventbridge_detail_json", ("http.request.body",)),
    ),
    *gap(
        "redis replies with bytes keys, such as HGETALL, aren't masked by key",
        Case("GAP-PY-6", "redis_hgetall_bytes_keys", ("db.response.body",)),
    ),
    *gap(
        "the redis command is a list of words, not keys, so only `all` masks it",
        Case("GAP-PY-15", "redis_statement", ("db.statement", "db.request.body")),
    ),
    *gap(
        "the psycopg2 instrumentation turns on capture_parameters for every DB-API "
        "driver, and the parameters aren't masked",
        Case("GAP-PY-7", "dbapi_statement_parameters", ("db.statement.parameters",)),
    ),
    *gap(
        "values written into the SQL text stay in db.statement",
        Case("GAP-PY-16", "dbapi_inline_sql_values", ("db.statement",)),
    ),
    *gap(
        "gRPC request and response payloads aren't masked",
        Case("GAP-PY-8", "grpc_payloads", ("rpc.grpc.",)),
    ),
    *gap(
        "span event attributes, such as exception.message, are never masked",
        Case("GAP-PY-11", "flask_exception_message", ("exception.",), events=True),
    ),
    *gap(
        "Cookie and Set-Cookie headers don't match the default patterns",
        Case(
            "GAP-PY-12",
            "requests_cookie_header",
            ("http.request.headers", "http.response.headers"),
        ),
        Case(
            "GAP-PY-12",
            "flask_cookie_header",
            ("http.request.headers", "http.response.headers"),
        ),
    ),
    *gap(
        "attributes the user opts in to upstream are set without masking",
        Case(
            "GAP-PY-13",
            "flask_opt_in_header_capture",
            ("http.request.header.",),
            env=(
                (
                    "OTEL_INSTRUMENTATION_HTTP_CAPTURE_HEADERS_SERVER_REQUEST",
                    "authorization",
                ),
            ),
        ),
        Case(
            "GAP-PY-13",
            "flask_query",
            ("url.query",),
            env=(("OTEL_SEMCONV_STABILITY_OPT_IN", "http"),),
        ),
    ),
    # A JSON body streamed in chunks is joined before it is masked
    *not_reproduced(
        Case("GAP-PY-14", "fastapi_chunked_json_body", ("http.request.body",)),
    ),
]


def secret_of(case: Case) -> str:
    return f"s3cr3t-{case.gap.lower()}"


@lru_cache(maxsize=None)
def _capture(case: Case) -> List[Dict[str, Any]]:
    mapped, _ = map_masking_env({})
    process_env = {k: v for k, v in os.environ.items() if not k.startswith(MASKING_ENV)}
    process_env.update(mapped)
    process_env.update(case.env)
    process_env.update(
        {
            "PYTHONPATH": os.pathsep.join([str(DISTRO_ROOT / "src"), str(CAPTURE.parent)]),
            "DASH0_SWITCH_OFF": "true",
            "DASH0_DISABLE_PYTHON_DEPENDENCY_CHECK": "true",
        }
    )
    result = subprocess.run(
        [sys.executable, str(CAPTURE), case.scenario, secret_of(case)],
        env=process_env,
        cwd=DISTRO_ROOT,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        # Not an AssertionError, so a broken scenario never counts as a reproduced gap
        raise RuntimeError(result.stderr.decode())
    return json.loads(result.stdout)  # type: ignore


@pytest.fixture(scope="module", autouse=True)
def run_scenarios_in_parallel() -> None:
    with ThreadPoolExecutor(max_workers=8) as pool:
        list(pool.map(_capture, [p.values[0] for p in CASES]))


def forms_of(secret: str) -> List[str]:
    """The secret as text, and as the list of byte values aws_dump makes of bytes."""
    return [secret, ", ".join(str(b) for b in secret.encode())]


class TestKnownMaskingGaps:
    @pytest.mark.parametrize("case", CASES)
    def test_secret_is_not_exported(self, case: Case) -> None:
        spans = _capture(case)
        checked: Dict[str, Any] = {}
        for span in spans:
            attributes = dict(span["attributes"])
            if case.events:
                for event in span["events"]:
                    attributes.update(event["attributes"])
            for name, value in attributes.items():
                if name.startswith(case.attributes):
                    checked[f"{span['name']}: {name}"] = value
        if not checked:
            # Not an AssertionError, so it fails the test even under xfail: a scenario
            # that captures nothing would pass without testing anything.
            pytest.fail(f"{case.scenario} captured none of {case.attributes}")
        leaked = {
            name: value
            for name, value in checked.items()
            if any(form in str(value) for form in forms_of(secret_of(case)))
        }
        assert not leaked, f"{case.gap}: the secret is exported in {leaked}"

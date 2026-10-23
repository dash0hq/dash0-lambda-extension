"""
Safeguards that keep masking aligned with what the instrumentations capture.

- Every instrumentation the distro installs must be classified below: its payloads are
  covered by the scenarios in test_masking_env.py, or masked by its hooks, or listed as not
  masked in docs/masking.md, or it can't be installed. A new instrumentation fails here until
  someone decides which.
- Every payload attribute a hook sets must get its value from the masking functions. A new
  `span.set_attribute("x.body", raw)` fails here.

test_masking_env.py checks what the masking does; this checks that nothing bypasses it.
"""

import ast
import importlib.util
import re
from pathlib import Path
from typing import Dict, Iterator, List, Optional, Set, Tuple

import pytest

from test_masking_env import CAPTURED, REPO_ROOT

INSTRUMENTATIONS = (
    Path(__file__).resolve().parents[3] / "dash0_opentelemetry" / "instrumentations"
)

SCENARIO = "payloads covered by test_masking_env.py"
MASKED_BY_HOOKS = "payloads masked by its hooks, checked statically below"
DOCUMENTED = "payloads listed as not masked in docs/masking.md"
NOT_INSTALLABLE = "the OpenTelemetry instrumentation isn't a dependency of the layer"

# instrumentations/<module> -> (classification, detail). For SCENARIO, detail is the target
# in test_masking_env.py; for DOCUMENTED, text that the docs' Python "not masked" table
# must contain; for NOT_INSTALLABLE, the module that must stay missing.
COVERAGE: Dict[str, Tuple[str, str]] = {
    "botocore": (SCENARIO, "sqs"),
    "django": (SCENARIO, "django"),
    "fastapi": (SCENARIO, "fastapi"),
    "flask": (SCENARIO, "flask"),
    "httpx": (SCENARIO, "httpx"),
    "requests": (SCENARIO, "requests"),
    "redis": (SCENARIO, "redis"),
    "kafka_python": (SCENARIO, "kafka"),
    "pika": (SCENARIO, "pika"),
    # db.request.body and db.response.body go through dump_with_context
    "pymongo": (MASKED_BY_HOOKS, ""),
    # results are masked; query parameters are captured by the upstream instrumentation
    "psycopg": (DOCUMENTED, "`psycopg`"),
    "psycopg2": (DOCUMENTED, "`psycopg`"),
    "grpcio": (DOCUMENTED, "gRPC"),
    "pymysql": (DOCUMENTED, "`pymysql`"),
    "langchain": (DOCUMENTED, "`langchain`"),
    "agno": (DOCUMENTED, "`agno`"),
    "boto": (NOT_INSTALLABLE, "opentelemetry.instrumentation.boto"),
}

# Targets of test_masking_env.py that are driven through hooks rather than a real client
STRING_PAYLOAD_TARGETS = {"redis", "kafka", "pika"}

# Attributes that hold a payload, by name
PAYLOAD_ATTRIBUTE = re.compile(
    r"\.(body|headers|query|query_string|payload|parameters)$|^db\.statement$"
)

# Functions whose result is masked
MASKING_FUNCTIONS = {
    "dump",
    "dump_with_context",
    "mask_and_dump",
    "mask_text",
    "safe_dump_payload",
    "safe_extract_http_request_body",
}

CONTEXTS = {
    "requestBody",
    "responseBody",
    "requestHeaders",
    "responseHeaders",
    "queryParams",
}

# (file, attribute) pairs that may be set without masking, and why
UNMASKED_ATTRIBUTES = {
    ("pymongo/__init__.py", "db.statement"): "the command name only, such as `find`",
    (
        "grpcio/grpc_instrument_client.py",
        "rpc.grpc.request.payload",
    ): "gRPC, documented",
    (
        "grpcio/grpc_instrument_client.py",
        "rpc.grpc.response.payload",
    ): "gRPC, documented",
    (
        "grpcio/grpc_instrument_server.py",
        "rpc.grpc.request.payload",
    ): "gRPC, documented",
}

# Files that set attributes whose names are only known at runtime, and why
DYNAMIC_ATTRIBUTE_FILES = {
    "instrumentation_utils.py": "add_body_attribute, which masks its value",
    "grpcio/common.py": "gRPC payloads, documented",
}


def installed_instrumentations() -> Set[str]:
    tree = ast.parse((INSTRUMENTATIONS / "instrumentations.py").read_text())
    return {
        node.module
        for node in ast.walk(tree)
        if isinstance(node, ast.ImportFrom) and node.level == 1 and node.module
    }


def python_not_masked_section() -> str:
    docs = (REPO_ROOT / "docs" / "masking.md").read_text()
    section = docs.split("## Payloads that are not masked")[1]
    return section.split("**Python**")[1].split("**Java**")[0]


class TestEveryInstrumentationIsClassified:
    def test_every_installed_instrumentation_is_classified(self) -> None:
        installed = installed_instrumentations()
        assert installed == set(COVERAGE), (
            "Classify each instrumentation in COVERAGE, and update docs/masking.md if it "
            f"captures payloads that aren't masked. Missing: {installed - set(COVERAGE)}, "
            f"stale: {set(COVERAGE) - installed}"
        )

    @pytest.mark.parametrize(
        "module,target", [(m, d) for m, (c, d) in COVERAGE.items() if c == SCENARIO]
    )
    def test_scenario_targets_exist(self, module: str, target: str) -> None:
        assert target in CAPTURED or target in STRING_PAYLOAD_TARGETS, module

    @pytest.mark.parametrize(
        "module,text", [(m, d) for m, (c, d) in COVERAGE.items() if c == DOCUMENTED]
    )
    def test_documented_ones_are_in_the_docs(self, module: str, text: str) -> None:
        assert text in python_not_masked_section(), module

    @pytest.mark.parametrize(
        "module,dependency",
        [(m, d) for m, (c, d) in COVERAGE.items() if c == NOT_INSTALLABLE],
    )
    def test_not_installable_ones_stay_missing(
        self, module: str, dependency: str
    ) -> None:
        # Once it is a dependency, it captures data: classify it again.
        assert importlib.util.find_spec(dependency) is None, module


def _name_of(node: ast.expr) -> Optional[str]:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return node.attr
    return None


def _masked_names(function: ast.AST) -> Set[str]:
    """Variables assigned from a masking function, in one function."""
    names = set()
    for node in ast.walk(function):
        if isinstance(node, ast.Assign) and _is_masked_call(node.value, set()):
            names.update(t.id for t in node.targets if isinstance(t, ast.Name))
    return names


def _is_masked_call(value: ast.expr, masked_names: Set[str]) -> bool:
    if isinstance(value, ast.Name):
        return value.id in masked_names
    return isinstance(value, ast.Call) and _name_of(value.func) in MASKING_FUNCTIONS


def payload_assignments(
    path: Path,
) -> Iterator[Tuple[int, Optional[str], Optional[ast.expr], Set[str]]]:
    """
    Yields (line, attribute name or None if dynamic, value, masked names in scope) for every
    attribute that a file sets, through set_attribute, set_attributes, a dict literal or a
    subscript assignment.
    """
    tree = ast.parse(path.read_text())
    scopes: List[ast.AST] = [
        n
        for n in ast.walk(tree)
        if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))
    ]
    for scope in scopes:
        masked = _masked_names(scope)
        for node in ast.walk(scope):
            if isinstance(node, ast.Call) and _name_of(node.func) == "set_attribute":
                name = node.args[0] if node.args else None
                value = node.args[1] if len(node.args) > 1 else None
                if isinstance(name, ast.Constant) and isinstance(name.value, str):
                    yield node.lineno, name.value, value, masked
                else:
                    yield node.lineno, None, value, masked
            elif isinstance(node, ast.Dict):
                for key, value in zip(node.keys, node.values):
                    if isinstance(key, ast.Constant) and isinstance(key.value, str):
                        yield node.lineno, key.value, value, masked
            elif isinstance(node, ast.Assign):
                for target in node.targets:
                    if (
                        isinstance(target, ast.Subscript)
                        and isinstance(target.slice, ast.Constant)
                        and isinstance(target.slice.value, str)
                    ):
                        yield node.lineno, target.slice.value, node.value, masked


def hook_files() -> List[Path]:
    return sorted(INSTRUMENTATIONS.rglob("*.py"))


def relative(path: Path) -> str:
    return path.relative_to(INSTRUMENTATIONS).as_posix()


class TestNothingBypassesMasking:
    def test_payload_attributes_are_set_from_masked_values(self) -> None:
        unmasked = []
        for path in hook_files():
            for line, name, value, masked in payload_assignments(path):
                if name is None or not PAYLOAD_ATTRIBUTE.search(name):
                    continue
                if (relative(path), name) in UNMASKED_ATTRIBUTES:
                    continue
                if value is None or not _is_masked_call(value, masked):
                    unmasked.append(f"{relative(path)}:{line} {name}")
        # Nested hooks are walked from each enclosing function, so drop duplicates.
        unmasked = sorted(set(unmasked))
        assert not unmasked, (
            "Set these from dump_with_context or add_body_attribute(..., context), or "
            f"list them in UNMASKED_ATTRIBUTES and docs/masking.md: {unmasked}"
        )

    def test_attribute_names_are_known_unless_reviewed(self) -> None:
        dynamic = {
            relative(path)
            for path in hook_files()
            for _, name, _, _ in payload_assignments(path)
            if name is None
        }
        assert dynamic <= set(DYNAMIC_ATTRIBUTE_FILES), dynamic - set(
            DYNAMIC_ATTRIBUTE_FILES
        )

    def test_add_body_attribute_always_names_a_payload_type(self) -> None:
        missing = []
        for path in hook_files():
            for node in ast.walk(ast.parse(path.read_text())):
                if (
                    isinstance(node, ast.Call)
                    and _name_of(node.func) == "add_body_attribute"
                ):
                    context = node.args[3] if len(node.args) > 3 else None
                    for keyword in node.keywords:
                        if keyword.arg == "context":
                            context = keyword.value
                    if not (
                        isinstance(context, ast.Constant) and context.value in CONTEXTS
                    ):
                        missing.append(f"{relative(path)}:{node.lineno}")
        assert not missing, missing

    def test_exemptions_are_still_needed(self) -> None:
        used = {
            (relative(path), name)
            for path in hook_files()
            for _, name, _, _ in payload_assignments(path)
        }
        assert set(UNMASKED_ATTRIBUTES) <= used, set(UNMASKED_ATTRIBUTES) - used
        files = {relative(p) for p in hook_files()}
        assert set(DYNAMIC_ATTRIBUTE_FILES) <= files

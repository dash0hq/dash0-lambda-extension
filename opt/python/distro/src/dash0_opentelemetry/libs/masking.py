"""
Secret masking for the payloads the distro captures.

`opt/shared.sh` maps the documented DASH0_MASK_* variables onto LUMIGO_SECRET_MASKING_REGEX*
before the runtime starts. lumigo_core reads the same variables, but it anchors patterns at
the start of the key, fails on `all` in LUMIGO_SECRET_MASKING_REGEX, and caches what it read.
This module masks the way the Dash0 extension and the Node distro do instead: a pattern
matches anywhere in a key, ignoring case, and a matching key masks its whole value. The
variables are read on every call; only parsing a given value is cached.
"""

import json
import logging
import math
import os
import re
from collections.abc import Mapping
from contextlib import contextmanager
from functools import lru_cache
from typing import Any, Dict, Iterator, List, Optional, Pattern, Tuple, Union
from urllib.parse import parse_qsl

from lumigo_core.lumigo_utils import aws_dump

from dash0_opentelemetry import logger

MASKED = "****"
TRUNCATE_SUFFIX = "...[too long]"

# Must match DEFAULT_MASKING_PATTERNS in src/otlp/masking.rs and DASH0_DEFAULT_MASK_RULES in
# opt/shared.sh. Used when the wrapper didn't run, or LUMIGO_* holds no valid value.
DEFAULT_MASKING_PATTERNS = [
    ".*pass.*",
    ".*key.*",
    ".*secret.*",
    ".*credential.*",
    ".*passphrase.*",
    ".*token.*",
    ".*signature.*",
    ".*authorization.*",
]

RULES_ENV = "LUMIGO_SECRET_MASKING_REGEX"
LEGACY_RULES_ENV = "LUMIGO_BLACKLIST_REGEX"
CONTEXT_ENVS = {
    "requestBody": "LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES",
    "responseBody": "LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES",
    "requestHeaders": "LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_HEADERS",
    "responseHeaders": "LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_HEADERS",
    "queryParams": "LUMIGO_SECRET_MASKING_REGEX_HTTP_QUERY_PARAMS",
    "environment": "LUMIGO_SECRET_MASKING_REGEX_ENVIRONMENT",
}


class MaskingRule:
    def __init__(self, patterns: Tuple[str, ...] = (), mask_all: bool = False):
        self.patterns = patterns
        self.mask_all = mask_all
        self._regex: Optional[Pattern[str]] = None
        self._regexes: Tuple[Pattern[str], ...] = ()
        if patterns:
            searched = [_without_outer_wildcards(p) for p in patterns]
            try:
                # One regex is much faster than one per pattern, on every key.
                self._regex = re.compile(
                    "|".join(f"(?:{p})" for p in searched), re.IGNORECASE
                )
            except re.error:
                # e.g. a pattern with inline flags, which must start the whole regex
                self._regexes = tuple(re.compile(p, re.IGNORECASE) for p in searched)

    def matches(self, key: str) -> bool:
        if self.mask_all:
            return True
        if self._regex is not None:
            return self._regex.search(key) is not None
        return any(r.search(key) for r in self._regexes)


def _without_outer_wildcards(pattern: str) -> str:
    """
    Drops a leading and a trailing `.*`, which don't change whether `search` finds a match
    (`.*` can match nothing), but make it retry the pattern from every position in the key.
    `.*pass.*` becomes `pass`.
    """
    stripped = pattern
    if stripped.startswith(".*") and stripped[2:3] not in ("?", "+", "{"):
        stripped = stripped[2:]
    if stripped.endswith(".*"):
        # `\.*` is an escaped dot, not a wildcard
        backslashes = len(stripped[:-2]) - len(stripped[:-2].rstrip("\\"))
        if backslashes % 2 == 0:
            stripped = stripped[:-2]
    try:
        re.compile(stripped)
        return stripped
    except re.error:
        return pattern


def _compile(patterns: List[str]) -> MaskingRule:
    valid = []
    for pattern in patterns:
        try:
            re.compile(pattern)
            valid.append(pattern)
        except re.error:
            # Dropped without failing the other patterns, like the extension does.
            logger.debug(f"Ignoring invalid masking pattern {pattern!r}")
    return MaskingRule(tuple(valid))


DEFAULT_RULE = _compile(DEFAULT_MASKING_PATTERNS)


@lru_cache(maxsize=64)
def _parse_rule(env_name: str, value: str) -> Optional[MaskingRule]:
    """Returns the rule for a variable's value, or None if the value is invalid."""
    if value.strip().lower() == "all":
        return MaskingRule(mask_all=True)
    try:
        patterns = json.loads(value)
    except ValueError:
        patterns = None
    if not isinstance(patterns, list) or not all(isinstance(p, str) for p in patterns):
        logger.warning(
            f'Ignoring {env_name}: expected a JSON array of regular expressions or "all"'
        )
        return None
    return _compile(patterns)


def get_rule(context: Optional[str] = None) -> MaskingRule:
    """
    The rule for a payload type: its own variable if set and valid, then the global rules,
    then the defaults.
    """
    for env_name in (CONTEXT_ENVS.get(context or ""), RULES_ENV, LEGACY_RULES_ENV):
        value = os.environ.get(env_name) if env_name else None
        if value:
            rule = _parse_rule(env_name, value)
            if rule is not None:
                return rule
    return DEFAULT_RULE


def mask(value: Any, rule: MaskingRule) -> Any:
    """Masks the value of every key the rule matches, at any depth. Values are never checked."""
    return _mask(value, rule, math.inf)[0]


def _mask(value: Any, rule: MaskingRule, budget: float) -> Tuple[Any, float]:
    """
    Like `mask`, but stops adding keys and items once about `budget` characters of output
    are used, since the result is truncated anyway. Returns the remaining budget, which is
    negative if anything was left out.
    """
    # Exact type checks first: an ABC isinstance check costs several times as much, on
    # every value, and payloads are mostly built of dicts, lists and strings.
    value_type = type(value)
    if value_type is str:
        return value, budget - len(value)
    if value_type is dict or (value_type is not list and isinstance(value, Mapping)):
        masked: Dict[Any, Any] = {}
        for k, v in value.items():
            if budget < 0:
                break
            if isinstance(k, str) and rule.matches(k):
                masked[k] = MASKED
                budget -= len(k) + 10
            else:
                masked[k], budget = _mask(v, rule, budget - len(str(k)) - 4)
        return masked, budget
    if value_type is list or isinstance(value, tuple):
        items = []
        for v in value:
            if budget < 0:
                break
            item, budget = _mask(v, rule, budget - 2)
            items.append(item)
        return items, budget
    if isinstance(value, (str, int, float)):
        return value, budget - len(str(value))
    return value, budget - 8


def _dump_masked(payload: Any, rule: MaskingRule, max_size: int) -> str:
    masked, budget = _mask(payload, rule, max_size)
    try:
        dumped = aws_dump(masked)
    except TypeError:
        dumped = str(masked)
    if budget < 0 or len(dumped) >= max_size:
        return dumped[:max_size] + TRUNCATE_SUFFIX
    return dumped


def _parse_json_text(text: str) -> Any:
    """Parses text that holds a JSON object or array, so it can be masked by key."""
    if text.lstrip()[:1] in ("{", "["):
        try:
            return json.loads(text)
        except ValueError:
            pass
    return text


def mask_and_dump(payload: Any, context: Optional[str], max_size: int) -> str:
    """
    Masks a payload and serializes it as JSON. A string or bytes payload that holds a JSON
    object or array is parsed first; any other string is kept as a JSON string.
    """
    rule = get_rule(context)
    if rule.mask_all:
        return json.dumps(MASKED)
    if isinstance(payload, bytes):
        try:
            payload = payload.decode()
        except Exception:
            payload = str(payload)
    if isinstance(payload, str):
        payload = _parse_json_text(payload)
    return _dump_masked(payload, rule, max_size)


def mask_text(text: str, context: Optional[str], max_size: int) -> str:
    """
    Masks a payload captured as text. Text that holds a JSON object or array is masked by key
    and re-serialized; other text is returned as it is, or as "****" for `all`.
    """
    rule = get_rule(context)
    if rule.mask_all:
        return json.dumps(MASKED)
    parsed = _parse_json_text(text)
    if parsed is text:
        return text
    return _dump_masked(parsed, rule, max_size)


def parse_query_string(
    query: Union[str, bytes],
) -> Dict[str, Union[str, List[str]]]:
    """Parses a query string into a dict; a repeated parameter becomes a list."""
    if isinstance(query, bytes):
        query = query.decode("latin-1")
    params: Dict[str, Union[str, List[str]]] = {}
    for key, value in parse_qsl(query, keep_blank_values=True):
        if key not in params:
            params[key] = value
        else:
            existing = params[key]
            params[key] = (
                existing + [value] if isinstance(existing, list) else [existing, value]
            )
    return params


@contextmanager
def lumigo_config_logs_silenced() -> Iterator[None]:
    """
    Importing lumigo_core.configuration parses LUMIGO_SECRET_MASKING_REGEX_*, and logs a
    CRITICAL "shutting down the tracer" for a value it can't parse. The distro masks with this
    module instead, and nothing is shut down, so that message is misleading.
    """
    lumigo_logger = logging.getLogger("lumigo")
    disabled = lumigo_logger.disabled
    lumigo_logger.disabled = True
    try:
        yield
    finally:
        lumigo_logger.disabled = disabled

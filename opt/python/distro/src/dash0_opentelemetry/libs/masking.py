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
import os
import re
from contextlib import contextmanager
from functools import lru_cache
from typing import Any, Dict, Iterator, List, Mapping, Optional, Pattern, Tuple, Union
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
    def __init__(self, patterns: Tuple[Pattern[str], ...] = (), mask_all: bool = False):
        self.patterns = patterns
        self.mask_all = mask_all

    def matches(self, key: str) -> bool:
        return self.mask_all or any(p.search(key) for p in self.patterns)


def _compile(patterns: List[str]) -> MaskingRule:
    compiled = []
    for pattern in patterns:
        try:
            compiled.append(re.compile(pattern, re.IGNORECASE))
        except re.error:
            # Dropped without failing the other patterns, like the extension does.
            logger.debug(f"Ignoring invalid masking pattern {pattern!r}")
    return MaskingRule(tuple(compiled))


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
    if isinstance(value, Mapping):
        return {
            k: MASKED if isinstance(k, str) and rule.matches(k) else mask(v, rule)
            for k, v in value.items()
        }
    if isinstance(value, (list, tuple)):
        return [mask(v, rule) for v in value]
    return value


def _parse_json_text(text: str) -> Any:
    """Parses text that holds a JSON object or array, so it can be masked by key."""
    if text.lstrip()[:1] in ("{", "["):
        try:
            return json.loads(text)
        except ValueError:
            pass
    return text


def _truncate(text: str, max_size: int) -> str:
    return text[:max_size] + TRUNCATE_SUFFIX if len(text) >= max_size else text


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
    masked = mask(payload, rule)
    try:
        dumped = aws_dump(masked)
    except TypeError:
        dumped = str(masked)
    return _truncate(dumped, max_size)


def mask_text(text: str, context: Optional[str]) -> str:
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
    return aws_dump(mask(parsed, rule))


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

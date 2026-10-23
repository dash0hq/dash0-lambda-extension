from typing import Any, Optional

from dash0_opentelemetry.libs.general_utils import get_max_size
from dash0_opentelemetry.libs.masking import mask_and_dump


def dump(to_dump: Any, max_size: Optional[int] = None) -> str:
    """Masks with DASH0_MASK_RULES (or the defaults) and serializes."""
    return mask_and_dump(to_dump, None, max_size or get_max_size())


def dump_with_context(
    context: str,
    to_dump: Any,
) -> str:
    """Masks with the rules of the payload type `context`, and serializes."""
    return mask_and_dump(to_dump, context, get_max_size())


def safe_convert_bytes_to_string(value: Any) -> Any:
    try:
        return value.decode("utf-8")
    except Exception:
        return value

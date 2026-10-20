import json
from typing import Any, Mapping, Optional

from opentelemetry.trace.span import Span

from dash0_opentelemetry.libs.general_utils import dash0_safe_execute
from dash0_opentelemetry.libs.masking import MASKED, get_rule, mask_and_dump, mask_text

PAYLOAD_MAX_SIZE = 4 * 1024


def add_body_attribute(
    span: Span, body: Any, attr_name: str, context: Optional[str] = None
) -> None:
    """
    Sets a payload attribute, masked with the rules of the payload type `context`. Text that
    holds JSON is masked by key; other text is kept as it is, and only `all` masks it.
    """
    with dash0_safe_execute(f"set body attribute ({attr_name})"):
        if isinstance(body, bytes):
            try:
                body = body.decode("utf-8")
            except UnicodeDecodeError:
                # Binary data has no keys to mask; only `all` hides it.
                if get_rule(context).mask_all:
                    span.set_attribute(attr_name, json.dumps(MASKED))
                else:
                    span.set_attribute(attr_name, "0x" + body.hex()[:PAYLOAD_MAX_SIZE])
                return
        if isinstance(body, str):
            value = mask_text(body, context)
        elif isinstance(body, (Mapping, list, tuple)):
            value = mask_and_dump(body, context, PAYLOAD_MAX_SIZE)
        else:
            value = mask_text(str(body), context)

        span.set_attribute(attr_name, value[:PAYLOAD_MAX_SIZE])

import logging

import pytest

from dash0_opentelemetry import logger
from dash0_opentelemetry.libs.masking import _parse_rule


@pytest.fixture(autouse=True)
def clear_cache():
    _parse_rule.cache_clear()


@pytest.fixture(autouse=True)
def capture_all_logs(caplog):
    logger.setLevel(logging.DEBUG)
    logger.propagate = True
    caplog.set_level(logging.DEBUG, logger="lumigo")

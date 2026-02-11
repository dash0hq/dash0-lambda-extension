import pytest

from dash0_opentelemetry.utils.config import (
    get_disabled_instrumentations,
)


def test_get_disabled_instrumentations_default():
    """Test default behavior when environment variable is not set"""
    assert get_disabled_instrumentations() == set()


def test_get_disabled_instrumentations_empty(monkeypatch):
    """Test behavior when environment variable is empty"""
    monkeypatch.setenv("DASH0_DISABLE_INSTRUMENTATION", "")
    assert get_disabled_instrumentations() == set()


def test_get_disabled_instrumentations_single(monkeypatch):
    """Test behavior with a single instrumentation"""
    monkeypatch.setenv("DASH0_DISABLE_INSTRUMENTATION", "boto")
    assert get_disabled_instrumentations() == {"boto"}


def test_get_disabled_instrumentations_multiple(monkeypatch):
    """Test behavior with multiple instrumentations"""
    monkeypatch.setenv("DASH0_DISABLE_INSTRUMENTATION", "boto,requests,redis")
    assert get_disabled_instrumentations() == {"boto", "requests", "redis"}


def test_get_disabled_instrumentations_with_spaces(monkeypatch):
    """Test behavior with spaces around instrumentation names"""
    monkeypatch.setenv("DASH0_DISABLE_INSTRUMENTATION", " boto , requests , redis ")
    assert get_disabled_instrumentations() == {"boto", "requests", "redis"}


def test_get_disabled_instrumentations_with_empty_values(monkeypatch):
    """Test behavior with empty values in the list"""
    monkeypatch.setenv("DASH0_DISABLE_INSTRUMENTATION", "boto,,requests,")
    assert get_disabled_instrumentations() == {"boto", "requests"}


def test_get_disabled_instrumentations_mixed_case(monkeypatch):
    """Test behavior with mixed case (should preserve original case)"""
    monkeypatch.setenv("DASH0_DISABLE_INSTRUMENTATION", "Boto,REQUESTS,Redis")
    assert get_disabled_instrumentations() == {"Boto", "REQUESTS", "Redis"}

import sys
import unittest

from unittest import TestCase
from unittest.mock import patch, Mock

from opentelemetry.sdk.trace import SpanProcessor

from dash0_opentelemetry import init


class TestDistroInit(unittest.TestCase):
    def test_access_trace_providers(self):
        from dash0_opentelemetry import tracer_provider

        self.assertIsNotNone(tracer_provider)
        self.assertTrue(hasattr(tracer_provider, "force_flush"))
        self.assertTrue(hasattr(tracer_provider, "shutdown"))


class TestPythonVersionCheck(TestCase):
    @patch("sys.version_info", Mock())
    def test_python_version_too_old(self):
        # Mock version_info for Python 3.8
        sys.version_info.major = 3
        sys.version_info.minor = 8

        with self.assertLogs("dash0-opentelemetry", level="WARNING") as cm:
            result = init()

        self.assertEqual(result, {})
        self.assertIn(
            "Unsupported Python version 3.8; only Python 3.9 to 3.14 are supported.",
            cm.output[0],
        )

    @patch("sys.version_info", Mock())
    def test_python_version_supported(self):
        # Mock version_info for Python 3.9
        sys.version_info.major = 3
        sys.version_info.minor = 9

        with self.assertLogs("dash0-opentelemetry", level="WARNING"):
            result = init()

        self.assertIsInstance(result, dict)

    @patch("sys.version_info", Mock())
    def test_python_version_314_supported(self):
        # Mock version_info for Python 3.14
        sys.version_info.major = 3
        sys.version_info.minor = 14

        with self.assertLogs("dash0-opentelemetry", level="WARNING"):
            result = init()

        self.assertIsInstance(result, dict)

    @patch("sys.version_info", Mock())
    def test_python_version_too_new(self):
        # Mock version_info for Python 3.15
        sys.version_info.major = 3
        sys.version_info.minor = 15

        with self.assertLogs("dash0-opentelemetry", level="WARNING") as cm:
            result = init()

        self.assertEqual(result, {})
        self.assertIn(
            "Unsupported Python version 3.15; only Python 3.9 to 3.14 are supported.",
            cm.output[0],
        )

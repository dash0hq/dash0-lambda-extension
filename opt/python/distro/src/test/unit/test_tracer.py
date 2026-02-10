import sys
import unittest
from collections import namedtuple

from unittest import TestCase
from unittest.mock import patch, Mock

from opentelemetry.sdk.trace import SpanProcessor

from dash0_opentelemetry import init

VersionInfo = namedtuple("version_info", ["major", "minor", "micro", "releaselevel", "serial"])


class TestDistroInit(unittest.TestCase):
    def test_access_trace_providers(self):
        from dash0_opentelemetry import tracer_provider

        self.assertIsNotNone(tracer_provider)
        self.assertTrue(hasattr(tracer_provider, "force_flush"))
        self.assertTrue(hasattr(tracer_provider, "shutdown"))


class TestPythonVersionCheck(TestCase):
    @patch("sys.version_info", VersionInfo(3, 8, 0, "final", 0))
    def test_python_version_too_old(self):
        with self.assertLogs("dash0-opentelemetry", level="WARNING") as cm:
            result = init()

        self.assertEqual(result, {})
        self.assertIn(
            "Unsupported Python version 3.8; only Python 3.9 to 3.14 are supported.",
            cm.output[0],
        )

    @patch("sys.version_info", VersionInfo(3, 9, 0, "final", 0))
    def test_python_version_supported(self):
        with self.assertLogs("dash0-opentelemetry", level="INFO"):
            result = init()

        self.assertIsInstance(result, dict)

    @patch("sys.version_info", VersionInfo(3, 14, 0, "final", 0))
    def test_python_version_314_supported(self):
        with self.assertLogs("dash0-opentelemetry", level="INFO"):
            result = init()

        self.assertIsInstance(result, dict)

    @patch("sys.version_info", VersionInfo(3, 15, 0, "final", 0))
    def test_python_version_too_new(self):
        with self.assertLogs("dash0-opentelemetry", level="WARNING") as cm:
            result = init()

        self.assertEqual(result, {})
        self.assertIn(
            "Unsupported Python version 3.15; only Python 3.9 to 3.14 are supported.",
            cm.output[0],
        )

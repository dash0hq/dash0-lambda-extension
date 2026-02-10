import os
import tempfile
from unittest.mock import patch, MagicMock

import pytest

from dash0_opentelemetry.utils.check_dependencies import check_dependency_conflicts


def _write_requirements(tmp_path, lines):
    req_file = os.path.join(tmp_path, "requirements.txt")
    with open(req_file, "w") as f:
        f.write("\n".join(lines) + "\n")
    return req_file


def _mock_dist(name, version):
    dist = MagicMock()
    dist.metadata = {"Name": name, "Version": version}
    return dist


class TestCheckDependencyConflicts:
    def test_no_conflicts(self, tmp_path):
        req_file = _write_requirements(tmp_path, ["requests==2.31.0"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [_mock_dist("requests", "2.31.0")]
            mock_requires.return_value = None
            assert check_dependency_conflicts(req_file) is False

    def test_version_conflict(self, tmp_path):
        req_file = _write_requirements(tmp_path, ["protobuf>=4.0,<5.0"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists:
            mock_dists.return_value = [_mock_dist("protobuf", "5.28.0")]
            assert check_dependency_conflicts(req_file) is True

    def test_package_not_installed(self, tmp_path):
        """If a required package is not installed, no conflict is reported."""
        req_file = _write_requirements(tmp_path, ["some-package==1.0.0"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists:
            mock_dists.return_value = []
            assert check_dependency_conflicts(req_file) is False

    def test_recursive_conflict(self, tmp_path):
        """Conflict in a sub-dependency should be detected."""
        req_file = _write_requirements(tmp_path, ["opentelemetry-proto==1.39.1"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [
                _mock_dist("opentelemetry-proto", "1.39.1"),
                _mock_dist("protobuf", "3.20.0"),
            ]
            mock_requires.return_value = ["protobuf>=4.0"]
            assert check_dependency_conflicts(req_file) is True

    def test_recursive_no_conflict(self, tmp_path):
        """No conflict in sub-dependencies should return False."""
        req_file = _write_requirements(tmp_path, ["opentelemetry-proto==1.39.1"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [
                _mock_dist("opentelemetry-proto", "1.39.1"),
                _mock_dist("protobuf", "5.28.0"),
            ]
            mock_requires.return_value = ["protobuf>=4.0"]
            assert check_dependency_conflicts(req_file) is False

    def test_skips_comments_and_flags(self, tmp_path):
        req_file = _write_requirements(tmp_path, [
            "# this is a comment",
            "-r other.txt",
            "requests==2.31.0",
            "",
            "  ",
        ])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [_mock_dist("requests", "2.31.0")]
            mock_requires.return_value = None
            assert check_dependency_conflicts(req_file) is False

    def test_skips_extras(self, tmp_path):
        req_file = _write_requirements(tmp_path, ['some-pkg[extra]==1.0.0'])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists:
            mock_dists.return_value = [_mock_dist("some-pkg", "2.0.0")]
            # "extra" is in the requirement string, so it should be skipped entirely
            assert check_dependency_conflicts(req_file) is False

    def test_missing_requirements_file(self):
        assert check_dependency_conflicts("/nonexistent/requirements.txt") is True

    def test_multiple_requirements_first_conflicts(self, tmp_path):
        req_file = _write_requirements(tmp_path, [
            "protobuf>=4.0,<5.0",
            "requests==2.31.0",
        ])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists:
            mock_dists.return_value = [
                _mock_dist("protobuf", "5.28.0"),
                _mock_dist("requests", "2.31.0"),
            ]
            assert check_dependency_conflicts(req_file) is True

    def test_multiple_requirements_no_conflicts(self, tmp_path):
        req_file = _write_requirements(tmp_path, [
            "protobuf>=4.0",
            "requests>=2.0",
        ])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [
                _mock_dist("protobuf", "5.28.0"),
                _mock_dist("requests", "2.31.0"),
            ]
            mock_requires.return_value = None
            assert check_dependency_conflicts(req_file) is False

    def test_compatible_version_specifier(self, tmp_path):
        req_file = _write_requirements(tmp_path, ["asgiref~=3.0"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [_mock_dist("asgiref", "3.7.2")]
            mock_requires.return_value = None
            assert check_dependency_conflicts(req_file) is False

    def test_compatible_version_specifier_conflict(self, tmp_path):
        req_file = _write_requirements(tmp_path, ["asgiref~=3.0"])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists:
            mock_dists.return_value = [_mock_dist("asgiref", "4.0.0")]
            assert check_dependency_conflicts(req_file) is True

    def test_git_url_requirement_skipped(self, tmp_path):
        """Git URL requirements (with @) should still be parseable by Requirement."""
        req_file = _write_requirements(tmp_path, [
            "opentelemetry-exporter-otlp-proto-http @ git+https://github.com/example/repo.git@branch#subdirectory=exporter",
        ])
        with patch("dash0_opentelemetry.utils.check_dependencies.distributions") as mock_dists, \
             patch("dash0_opentelemetry.utils.check_dependencies.requires") as mock_requires:
            mock_dists.return_value = [_mock_dist("opentelemetry-exporter-otlp-proto-http", "1.39.1")]
            mock_requires.return_value = None
            # URL requirements have no version specifier, so no conflict
            assert check_dependency_conflicts(req_file) is False

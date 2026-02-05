import logging
import os
from importlib import import_module
from importlib.metadata import distributions, requires, version

from packaging.requirements import Requirement

logger = logging.getLogger(__name__)


def check_dependency_conflicts(package_name):
    """Check for dependency conflicts between a package (and its sub-dependencies) and the current environment."""
    # Build a map of installed packages and their versions once
    installed = {dist.metadata["Name"].lower(): dist.metadata["Version"] for dist in distributions()}
    logger.warning(installed)

    visited = set()

    def check_package(pkg_name):
        pkg_name_lower = pkg_name.lower()
        if pkg_name_lower in visited:
            return
        visited.add(pkg_name_lower)

        try:
            package_requirements = requires(pkg_name)
            if not package_requirements:
                return

            logger.warning(f"Package '{pkg_name}' requirements: {package_requirements}")

            for req_str in package_requirements:
                # Skip extras (e.g., "package[extra]" or markers like "; extra == 'dev'")
                if "extra" in req_str:
                    continue

                try:
                    req = Requirement(req_str)
                    req_name = req.name.lower()

                    if req_name in installed:
                        installed_version = installed[req_name]
                        if not req.specifier.contains(installed_version):
                            logger.warning(
                                f"Dependency conflict: {pkg_name} requires {req_str}, "
                                f"but {req_name}=={installed_version} is installed"
                            )
                        # Recursively check sub-dependencies
                        check_package(req_name)
                except Exception as e:
                    logger.info(f"Could not parse requirement '{req_str}': {e}")

        except Exception as e:
            logger.info(f"Could not check dependencies for {pkg_name}: {e}")

    check_package(package_name)


check_dependency_conflicts("lumigo_opentelemetry")

import lumigo_opentelemetry
from opentelemetry.instrumentation.aws_lambda import AwsLambdaInstrumentor


def modify_module_name(module_name):
    """Returns a valid modified module to get imported"""
    return ".".join(module_name.split("/"))


class HandlerError(Exception):
    pass

path = os.environ.get("ORIG_HANDLER")

if path is None:
    raise HandlerError("ORIG_HANDLER is not defined.")

path = modify_module_name(path)
os.environ["ORIG_HANDLER"] = path

AwsLambdaInstrumentor().instrument()

try:
    (mod_name, handler_name) = path.rsplit(".", 1)
except ValueError as e:
    raise HandlerError("Bad path '{}' for ORIG_HANDLER: {}".format(path, str(e)))

handler_module = import_module(mod_name)
lambda_handler = getattr(handler_module, handler_name)
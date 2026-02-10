import logging
from importlib.metadata import distributions, requires, version
from packaging.requirements import Requirement

logger = logging.getLogger(__name__)


def check_dependency_conflicts(requirements_file):
    """Check for dependency conflicts between requirements and the current environment.

    Reads the top-level requirements from a file, then recursively checks
    sub-dependencies using importlib.metadata.

    Returns True if conflicts were found, False otherwise.
    """
    # Build a map of installed packages and their versions once
    installed = {dist.metadata["Name"].lower(): dist.metadata["Version"] for dist in distributions()}

    visited = set()

    def check_requirements(pkg_name, package_requirements):
        for req_str in package_requirements:
            if "extra" in req_str:
                continue

            try:
                req = Requirement(req_str)

                if req.marker and not req.marker.evaluate():
                    continue

                req_name = req.name.lower()

                if req_name in installed:
                    installed_version = installed[req_name]
                    if not req.specifier.contains(installed_version):
                        logger.warning(
                            f"Skipping instrumentation due to dependency conflict: {pkg_name} requires {req_str}, "
                            f"but {req_name}=={installed_version} is installed"
                        )
                        return True
                    # Recursively check sub-dependencies
                    if check_package(req_name):
                        return True
            except Exception as e:
                logger.warning(f"Could not parse requirement '{req_str}': {e}")
                return True

        return False

    def check_package(pkg_name):
        pkg_name_lower = pkg_name.lower()
        if pkg_name_lower in visited:
            return False
        visited.add(pkg_name_lower)

        try:
            package_requirements = requires(pkg_name)
            if not package_requirements:
                return False
            return check_requirements(pkg_name, package_requirements)
        except Exception as e:
            logger.warning(f"Could not check dependencies for {pkg_name}: {e}")
            return True

    # Read top-level requirements from file
    try:
        with open(requirements_file) as f:
            top_level_requirements = [line.strip() for line in f if line.strip() and not line.startswith(("#", "-"))]
    except Exception as e:
        logger.warning(f"Could not read requirements file {requirements_file}: {e}")
        return True

    return check_requirements("dash0_opentelemetry", top_level_requirements)
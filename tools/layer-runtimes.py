#!/usr/bin/env python3
"""Generate Lambda layer metadata from the canonical integration-test runtimes."""

import argparse
import ast
import json
from pathlib import Path
import re
import sys


def compatible_runtimes(layer, source):
    families = ("node", "python", "java") if layer == "manual" else (layer,)
    runtimes = []
    for family in families:
        match = re.search(
            rf"export const {family.upper()}_RUNTIMES\s*=\s*(\[[^\]]*\])\s*as const;",
            source,
        )
        if not match:
            raise ValueError(f"Missing {family.upper()}_RUNTIMES in runtimes.ts")
        runtimes.extend(runtime.replace("-", ".") for runtime in ast.literal_eval(match[1]))
    if layer == "manual":
        # No language distro: also make the layer discoverable for custom runtimes.
        runtimes.extend(("provided.al2", "provided.al2023"))
    runtimes = list(dict.fromkeys(runtimes))
    if not 1 <= len(runtimes) <= 15:
        raise ValueError(f"{layer}: Lambda requires 1–15 compatible runtimes, got {len(runtimes)}")
    return runtimes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("layer", choices=("node", "python", "java", "manual"))
    parser.add_argument("--check", action="store_true", help="Check GetLayerVersion JSON on stdin")
    args = parser.parse_args()
    source = (Path(__file__).resolve().parents[1] / "integration-tests/runtimes.ts").read_text()
    expected = compatible_runtimes(args.layer, source)
    if args.check:
        actual = json.load(sys.stdin)
        if not isinstance(actual, list) or sorted(actual) != sorted(expected):
            raise ValueError(f"{args.layer}: expected {expected}, got {actual}")
        print(f"Verified {args.layer} CompatibleRuntimes: {' '.join(actual)}")
    else:
        print(" ".join(expected))


if __name__ == "__main__":
    main()

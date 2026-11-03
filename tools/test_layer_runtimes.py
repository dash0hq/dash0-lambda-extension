import json
from pathlib import Path
import runpy
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).with_name("layer-runtimes.py")
compatible_runtimes = runpy.run_path(str(SCRIPT))["compatible_runtimes"]


class LayerRuntimesTest(unittest.TestCase):
    def test_publish_readback_contract(self):
        for layer in ("node", "python", "java", "manual"):
            with self.subTest(layer=layer):
                result = subprocess.run(
                    [sys.executable, str(SCRIPT), layer], check=True, capture_output=True, text=True,
                )
                runtimes = result.stdout.split()
                for actual, success in ((runtimes[::-1], True), (None, False), (runtimes[:-1], False)):
                    checked = subprocess.run(
                        [sys.executable, str(SCRIPT), layer, "--check"],
                        input=json.dumps(actual), capture_output=True, text=True,
                    )
                    self.assertEqual(checked.returncode == 0, success, checked.stderr)

    def test_family_selection_and_runtime_conversion(self):
        source = """
export const NODE_RUNTIMES = ['nodejs24-x', 'nodejs26-x'] as const;
export const PYTHON_RUNTIMES = ['python3-14'] as const;
export const JAVA_RUNTIMES = ['java25'] as const;
"""
        self.assertEqual(compatible_runtimes("node", source), ["nodejs24.x", "nodejs26.x"])
        self.assertEqual(compatible_runtimes("python", source), ["python3.14"])
        self.assertEqual(compatible_runtimes("java", source), ["java25"])
        self.assertEqual(compatible_runtimes("manual", source), [
            "nodejs24.x", "nodejs26.x", "python3.14", "java25", "provided.al2", "provided.al2023",
        ])

    def test_runtime_additions_and_api_limit(self):
        for count in (15, 16):
            source = f"export const NODE_RUNTIMES = {[f'nodejs{n}-x' for n in range(count)]} as const;"
            with self.subTest(count=count):
                if count == 15:
                    self.assertEqual(compatible_runtimes("node", source)[-1], "nodejs14.x")
                else:
                    with self.assertRaisesRegex(ValueError, "got 16"):
                        compatible_runtimes("node", source)

    def test_manual_limit_includes_custom_runtimes(self):
        source = "\n".join(
            f"export const {family}_RUNTIMES = {values!r} as const;"
            for family, values in (
                ("NODE", [f"nodejs{n}-x" for n in range(11)]),
                ("PYTHON", ["python3-14"]), ("JAVA", ["java25"]),
            )
        )
        self.assertEqual(len(compatible_runtimes("manual", source)), 15)
        with self.assertRaisesRegex(ValueError, "got 16"):
            compatible_runtimes("manual", source.replace("'java25'", "'java25', 'java29'"))

    def test_missing_runtime_list_fails(self):
        with self.assertRaisesRegex(ValueError, "Missing NODE_RUNTIMES"):
            compatible_runtimes("node", "")


if __name__ == "__main__":
    unittest.main()

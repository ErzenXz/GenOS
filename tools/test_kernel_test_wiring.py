"""Reject orphaned kernel test files that no longer compile with production modules."""

from pathlib import Path
import re
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[1]
CASES = {
    "kernel/src/exception/tests.rs": "exception::tests",
    "kernel/src/ipv6_tests.rs": "ipv6::tests",
    "kernel/src/network_tests.rs": "network_transport::network::tests",
    "kernel/src/storage_tests.rs": "storage_under_test::commit_tests",
}


def required_names() -> set[str]:
    names = set()
    for path, module in CASES.items():
        source = (ROOT / path).read_text()
        functions = re.findall(r"#\[test\]\s*fn\s+(\w+)\s*\(", source)
        if not functions:
            raise ValueError(f"no executable tests in {path}")
        names.update(f"{module}::{name}: test" for name in functions)
    return names


def missing_from(listing: str) -> set[str]:
    return required_names() - set(listing.splitlines())


class KernelTestWiringTests(unittest.TestCase):
    def test_all_standalone_cases_compile_from_production_modules(self):
        discovered = {
            path.relative_to(ROOT).as_posix()
            for path in (ROOT / "kernel/src").rglob("*test*.rs")
            if "#[test]" in path.read_text()
        }
        self.assertEqual(discovered, set(CASES))
        result = subprocess.run(["cargo", "test", "-p", "kernel", "--lib", "--", "--list"],
                                cwd=ROOT, capture_output=True, text=True, check=True,
                                timeout=120)
        self.assertGreaterEqual(len(required_names()), 48)
        self.assertEqual(missing_from(result.stdout), set())

    def test_a_detached_test_function_is_detected(self):
        name = next(iter(required_names()))
        self.assertIn(name, missing_from("not_that_function: test\n"))


if __name__ == "__main__":
    unittest.main()

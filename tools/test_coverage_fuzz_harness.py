"""Pinned source admission and retained coverage evidence contracts."""
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

from test_coverage_fuzz import build_runtime, check_source_files, parse_statistics, run

GOOD = "#2\tINITED cov: 3 ft: 4\n#100\tDONE cov: 8 ft: 9\nstat::number_of_executed_units: 100\nstat::new_units_added: 5\n"


class CoverageHarnessTests(unittest.TestCase):
    def test_missing_duplicate_wrong_count_and_zero_coverage_refuse(self):
        self.assertEqual(parse_statistics(GOOD, 100), {"coverage": 8, "features": 9, "new_units_added": 5})
        for line in GOOD.splitlines():
            for bad in (GOOD.replace(line + "\n", ""), GOOD + line + "\n"):
                with self.subTest(line=line), self.assertRaises(RuntimeError):
                    parse_statistics(bad, 100)
        for bad in (GOOD.replace("cov: 8", "cov: 0"), GOOD.replace("#100", "#99"),
                    GOOD.replace("units: 100", "units: 99"), GOOD.replace("DONE", "INITED")):
            with self.assertRaises(RuntimeError):
                parse_statistics(bad, 100)

    def test_unlocked_source_cannot_reach_a_compiler(self):
        with TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "Fuzzer.cpp").write_text("locked")
            lock = {"files": [{"name": "Fuzzer.cpp"}]}
            check_source_files(source, lock)
            (source / "Extra.cpp").write_text("unlocked")
            with patch("test_coverage_fuzz.run") as compiler, self.assertRaises(ValueError):
                build_runtime(source, lock, source / "evidence")
            compiler.assert_not_called()

    def test_symlinked_source_is_rejected(self):
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "cache"
            source.mkdir()
            outside = root / "outside.cpp"
            outside.write_text("unlocked")
            (source / "Fuzzer.cpp").symlink_to(outside)
            with self.assertRaises(ValueError):
                check_source_files(source, {"files": [{"name": "Fuzzer.cpp"}]})

    def test_timeout_preserves_partial_stdout(self):
        with TemporaryDirectory() as temporary:
            log = Path(temporary) / "partial.log"
            with self.assertRaises(subprocess.TimeoutExpired):
                run([sys.executable, "-c", "import time; print('partial',flush=True); time.sleep(10)"],
                    log=log, timeout=.3)
            self.assertIn("partial", log.read_text())


if __name__ == "__main__":
    unittest.main()

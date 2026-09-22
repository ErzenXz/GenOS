import tempfile
from pathlib import Path
import unittest

from test_memory_snapshot import ENTRY, READY, patch_fixture, validate_log


def valid_log():
    return "\n".join([
        "BOOT_MODE normal", "BOOT_MEMORY_MAP_VALIDATED", "GENOS_READY",
        "MEMORY_SNAPSHOT_BEGIN", "MEMORY_SNAPSHOT_LIVE phase=before frames=40",
        "MEMORY_SNAPSHOT_LIVE phase=during frames=53", "MEMORY_SNAPSHOT_FIRST_STABLE",
        "MEMORY_SNAPSHOT_LIVE phase=after frames=40", "MEMORY_SNAPSHOT_HANDLES_REUSED",
        READY, "NORMAL_SHELL_READY", "",
    ])


class MemorySnapshotHarnessTests(unittest.TestCase):
    def test_exact_proof_includes_live_allocation_and_cleanup(self):
        self.assertEqual(validate_log(valid_log()), {"before": 40, "during": 53, "after": 40})

    def test_missing_duplicate_embedded_and_unordered_records_fail(self):
        good = valid_log()
        for line in good.splitlines():
            for bad in [good.replace(line + "\n", ""), good + line + "\n",
                        good.replace(line, "prefix " + line),
                        good.replace(line + "\n", "") + line + "\n"]:
                if bad == good:
                    continue
                with self.subTest(line=line, log=bad), self.assertRaises(ValueError):
                    validate_log(bad)

    def test_false_pressure_cleanup_or_crashes_fail(self):
        good = valid_log()
        for bad in [good.replace("during frames=53", "during frames=40"),
                    good.replace("during frames=53", "during frames=39"),
                    good.replace("after frames=40", "after frames=41"),
                    good.replace("before frames=40", "before frames=0"),
                    good.replace("before frames=40", "before frames=-1"),
                    good.replace("BOOT_MODE normal", "BOOT_MODE validation"),
                    good + "MEMORY_SNAPSHOT_FAILED\n", good + "KERNEL PANIC\n",
                    good + "RECOVERY_CONSOLE_READY\n", good + "USER_FAULT_TERMINATED\n",
                    good + "MEMORY_SNAPSHOT_LIVE malformed\n"]:
            with self.subTest(log=bad), self.assertRaises(ValueError):
                validate_log(bad)

    def test_fixture_changes_only_shell_source_and_requires_the_entry_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shell = root / "userspace/shell/src/main.rs"
            fixture = root / "tools/fixtures/memory_snapshot.rs"
            shell.parent.mkdir(parents=True)
            fixture.parent.mkdir(parents=True)
            source = ENTRY + "\n    loop {}\n}\n"
            shell.write_text(source)
            fixture.write_text(source + "// disposable proof\n")
            patch = patch_fixture(root)
            self.assertEqual(shell.read_text(), fixture.read_text())
            self.assertIn("a/userspace/shell/src/main.rs", patch)
            self.assertNotIn("kernel/src", patch)
            shell.write_text("changed entry")
            with self.assertRaises(ValueError):
                patch_fixture(root)


if __name__ == "__main__":
    unittest.main()

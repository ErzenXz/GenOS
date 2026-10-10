import tempfile
from pathlib import Path
import unittest

from test_storage_startup import PROOFS, WRITE, FAIL_WRITE, patch_fixture, validate_log


class StorageStartupHarnessTests(unittest.TestCase):
    def test_exact_ordered_proof(self):
        validate_log("\n".join(PROOFS) + "\n")

    def test_missing_duplicate_embedded_and_reordered_proofs_fail(self):
        good = "\n".join(PROOFS) + "\n"
        for line in PROOFS:
            for bad in [good.replace(line + "\n", ""), good + line + "\n",
                        good.replace(line, "prefix " + line),
                        good.replace(line + "\n", "") + line + "\n"]:
                if bad != good:
                    with self.subTest(line=line, log=bad), self.assertRaises(ValueError):
                        validate_log(bad)

    def test_false_acknowledgment_and_faults_fail(self):
        good = "\n".join(PROOFS) + "\n"
        for forbidden in ["PERSISTENT_STORAGE_CREATED", "PERSISTENT_STORAGE_RESTORED",
                          "USER_STORAGE_FILE_COMMITTED", "USER_STORAGE_FILE_RESTORED",
                          "EXCEPTION_FRAME", "USER_FAULT_TERMINATED", "KERNEL PANIC",
                          "BOOTLOADER_PANIC", "RECOVERY_CONSOLE_READY", "HEADLESS_RUNTIME_FAILED",
                          "userspace lifecycle error"]:
            with self.subTest(forbidden=forbidden), self.assertRaises(ValueError):
                validate_log(good + forbidden + "\n")

    def test_fixture_changes_only_device_write_and_requires_exact_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "kernel/src/storage.rs"
            path.parent.mkdir(parents=True)
            before = "// commit and quarantine unchanged\n" + WRITE + "\n// policy unchanged\n"
            path.write_text(before)
            patch = patch_fixture(root)
            self.assertEqual(path.read_text(), before.replace(WRITE, FAIL_WRITE))
            self.assertIn("a/kernel/src/storage.rs", patch)
            self.assertNotIn("userspace/", patch)
            with self.assertRaises(ValueError):
                patch_fixture(root)
            path.write_text(before + WRITE)
            with self.assertRaises(ValueError):
                patch_fixture(root)


if __name__ == "__main__":
    unittest.main()

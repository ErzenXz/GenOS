import hashlib
import tempfile
import unittest
from pathlib import Path

from tools.audit_repeat_boots import AuditError, audit


COMMIT = "a" * 40


class RepeatAuditTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.evidence = self.root / "normal-evidence"
        self.artifacts = self.root / "artifacts"
        self.artifacts.mkdir()
        self.log = self.root / "repeat.log"
        for ordinal in range(1, 4):
            self.manifest(ordinal)
        self.log.write_text(
            "".join(
                f"repeat-{ordinal:04d} boot passed: visible workflow\n"
                f"NORMAL_BOOT_REPETITION_PROGRESS completed={ordinal} requested=3\n"
                for ordinal in range(1, 4)
            )
            + "NORMAL_BOOT_REPETITION_OK completed=3 requested=3\n",
            encoding="utf-8",
        )

    def manifest(self, ordinal, *, run_id=None, image="b" * 64, status="passed"):
        run_id = run_id or str(1000 + ordinal)
        path = self.evidence / run_id / f"repeat-{ordinal:04d}" / "manifest.txt"
        path.parent.mkdir(parents=True, exist_ok=True)
        serial = f"boot {ordinal}\n".encode()
        qemu = b""
        (self.artifacts / f"serial-repeat-{ordinal:04d}.log").write_bytes(serial)
        (self.artifacts / f"normal-repeat-{ordinal:04d}-qemu.log").write_bytes(qemu)
        path.write_text(
            "\n".join(
                (
                    f"status={status}",
                    f"run_id={run_id}",
                    "mode=Release",
                    "reference_profile=genos-q35-tcg-v3",
                    "reference_environment_match=true",
                    "profile_sha256=" + "c" * 64,
                    "firmware_sha256=" + "d" * 64,
                    "image_sha256=" + image,
                    "rust=rustc-test",
                    "qemu=qemu-test",
                    f"commit={COMMIT}",
                    'working_tree_status=""',
                    "serial_sha256=" + hashlib.sha256(serial).hexdigest(),
                    "qemu_log_sha256=" + hashlib.sha256(qemu).hexdigest(),
                )
            )
            + "\n",
            encoding="utf-8",
        )
        return path

    def check(self):
        return audit(self.evidence, self.log, COMMIT, 3, self.artifacts)

    def test_complete_clean_lane(self):
        self.assertEqual(self.check()["count"], "3")

    def test_duplicate_manifest_is_not_a_second_boot(self):
        self.manifest(2, run_id="2002")
        with self.assertRaisesRegex(AuditError, "duplicate boot 2"):
            self.check()

    def test_missing_manifest_is_not_hidden_by_log_marker(self):
        (self.evidence / "1002" / "repeat-0002" / "manifest.txt").unlink()
        with self.assertRaisesRegex(AuditError, r"missing=\[2\]"):
            self.check()

    def test_changed_image_is_rejected(self):
        self.manifest(3, image="0" * 64)
        with self.assertRaisesRegex(AuditError, "identity changed at boot 3"):
            self.check()

    def test_dirty_source_is_rejected(self):
        path = self.evidence / "1002" / "repeat-0002" / "manifest.txt"
        path.write_text(path.read_text().replace('working_tree_status=""', 'working_tree_status="M kernel"'))
        with self.assertRaisesRegex(AuditError, "dirty source at boot 2"):
            self.check()

    def test_failed_boot_is_reported_first(self):
        self.manifest(2, status="failed")
        with self.assertRaisesRegex(AuditError, "first failed boot 2"):
            self.check()

    def test_partial_log_cannot_pass_complete_manifests(self):
        self.log.write_text(self.log.read_text().replace("NORMAL_BOOT_REPETITION_OK completed=3 requested=3\n", ""))
        with self.assertRaisesRegex(AuditError, "completion marker missing"):
            self.check()

    def test_changed_serial_bytes_are_rejected(self):
        (self.artifacts / "serial-repeat-0002.log").write_bytes(b"forged\n")
        with self.assertRaisesRegex(AuditError, "boot-output hash mismatch at boot 2"):
            self.check()


if __name__ == "__main__":
    unittest.main()

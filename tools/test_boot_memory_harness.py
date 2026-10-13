from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import tarfile
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from test_boot_memory import CASES, LOADER_CASES, LOADER_ERRORS, loader_error_record, main, patch_fixture, validate_log


class BootMemoryEvidenceTests(unittest.TestCase):
    def test_exact_phase_and_rejection(self):
        for case in CASES:
            log = ("BOOT_MEMORY_MAP_REJECTED\n" if case in LOADER_CASES
                   else "GenOS kernel entered\nBOOT_INFO_REJECTED\n")
            if case in LOADER_ERRORS:
                log = loader_error_record(case) + "\n" + log
            validate_log(log, case)
            for forged in ("", log + log, "prefix " + log, log + "GENOS_READY\n",
                           log + "GDT/TSS initialized\n", log + "BOOT_MEMORY_MAP_VALIDATED\n"):
                with self.subTest(case=case, log=forged), self.assertRaises(ValueError):
                    validate_log(forged, case)

    def test_other_phase_cannot_substitute(self):
        with self.assertRaises(ValueError):
            validate_log("BOOT_MEMORY_MAP_REJECTED\n", "count")
        with self.assertRaises(ValueError):
            validate_log("GenOS kernel entered\nBOOT_MEMORY_MAP_REJECTED\n", "loader-version")

    def test_loader_error_must_match_case_and_precede_rejection(self):
        for case in LOADER_ERRORS:
            record = loader_error_record(case)
            good = record + "\nBOOT_MEMORY_MAP_REJECTED\n"
            for bad in ["BOOT_MEMORY_MAP_REJECTED\n", good + record + "\n",
                        "BOOT_MEMORY_MAP_REJECTED\n" + record + "\n",
                        good.replace(LOADER_ERRORS[case], "Wrong"),
                        good.replace(case, "other"), good + "BOOT_MEMORY_FIXTURE_WRONG_ERROR\n",
                        good + "BOOT_MEMORY_FIXTURE_INPUT_REJECTED\n"]:
                with self.subTest(case=case), self.assertRaises(ValueError):
                    validate_log(bad, case)
        with self.assertRaises(ValueError):
            validate_log("GenOS kernel entered\nBOOT_INFO_REJECTED\n", "unknown")

    def test_each_fixture_keeps_real_exit_and_decoder_and_uses_no_post_exit_allocation(self):
        original = (Path(__file__).resolve().parents[1] / "bootloader/src/main.rs").read_text()
        for case in LOADER_ERRORS:
            with TemporaryDirectory() as temporary:
                root = Path(temporary)
                path = root / "bootloader/src/main.rs"
                path.parent.mkdir(parents=True)
                path.write_text(original)
                patch = patch_fixture(root, case)
                changed = path.read_text()
                self.assertEqual(changed.count("unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) }"), 1)
                self.assertEqual(changed.count("boot_memory::decode_uefi_map("), 1)
                self.assertIn(f"Some(&boot_memory::BootMemoryError::{LOADER_ERRORS[case]})", changed)
                self.assertIn(loader_error_record(case), changed)
                self.assertIn("BOOT_MEMORY_FIXTURE_INPUT_REJECTED", changed)
                self.assertFalse(any(word in line for line in patch.splitlines() if line.startswith("+")
                                     for word in ["Vec", "allocate_pages", "boot::stall", "uefi::println"]))

    def test_loader_version_does_not_accept_a_different_case_error_tag(self):
        with self.assertRaises(ValueError):
            validate_log(loader_error_record("loader-empty") + "\nBOOT_MEMORY_MAP_REJECTED\n", "loader-version")

    def test_loader_group_retains_first_failure_and_still_attempts_every_later_case(self):
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            firmware = root / "firmware.fd"
            firmware.write_bytes(b"firmware")
            archive = io.BytesIO()
            with tarfile.open(fileobj=archive, mode="w"):
                pass

            def command(args, **kwargs):
                if args[0] == "git":
                    return SimpleNamespace(stdout=archive.getvalue())
                image = Path(kwargs["cwd"]) / "build/genos.img"
                image.parent.mkdir(parents=True)
                image.write_bytes(b"image")
                return SimpleNamespace(returncode=0)

            def boot_case(source, evidence, case, timeout):
                (evidence / "serial.log").write_text(case)
                if case == LOADER_CASES[0]:
                    raise TimeoutError("retained first failure")

            with patch("test_boot_memory.__file__", str(root / "tools/test_boot_memory.py")), \
                 patch("sys.argv", ["test_boot_memory.py", "--loader-admission"]), \
                 patch("test_boot_memory.require_clean_source", return_value="abc"), \
                 patch("test_boot_memory.firmware_path", return_value=str(firmware)), \
                 patch("test_boot_memory.environment", return_value={}), \
                 patch("test_boot_memory.patch_fixture", return_value="fixture"), \
                 patch("test_boot_memory.subprocess.run", side_effect=command), \
                 patch("test_boot_memory.boot", side_effect=boot_case) as boot, \
                 redirect_stdout(io.StringIO()), self.assertRaises(RuntimeError):
                main()
            self.assertEqual([call.args[2] for call in boot.call_args_list], list(LOADER_CASES))
            manifests = list((root / "build/boot-memory-evidence").glob("*/*/manifest.json"))
            self.assertEqual(len(manifests), len(LOADER_CASES))
            results = {json.loads(path.read_text())["case"]: json.loads(path.read_text()) for path in manifests}
            self.assertEqual(results[LOADER_CASES[0]]["status"], "failed")
            self.assertIn("retained first failure", results[LOADER_CASES[0]]["failure"])
            self.assertTrue(all(results[case]["status"] == "passed" for case in LOADER_CASES[1:]))
            self.assertTrue(all("serial_sha256" in result for result in results.values()))


if __name__ == "__main__":
    unittest.main()

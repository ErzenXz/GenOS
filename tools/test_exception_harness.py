"""Host-only regression tests for the fault probe's fail-closed evidence parser."""
import unittest

from test_exception_entry import USER_READY, patch_fixture, replace_once, validate_log
from pathlib import Path
from tempfile import TemporaryDirectory


def valid_log(mode="user", vector=6, error=0, cr2=0):
    cpl, cs, ss = (3, 0x33, 0x2b) if mode == "user" else (0, 8, 0x10)
    frame = (f"EXCEPTION_FRAME vector={vector} cpl={cpl} error=0x{error:x}"
             f" rip=0x400000 cs=0x{cs:x} rflags=0x202 rsp=0x500000"
             f" ss=0x{ss:x} cr2=0x{cr2:x}")
    lines = ["EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated", frame]
    if mode == "user":
        lines += [f"USER_FAULT_TERMINATED pid=1 vector={vector} error=0x{error:x}", *USER_READY]
    else:
        lines += ["KERNEL_EXCEPTION_PROBE_ARMED", "EXCEPTION_FATAL_HALT"]
    return "\n".join(lines) + "\n"


class EvidenceTests(unittest.TestCase):
    def test_all_eight_fault_identities_are_validated(self):
        for mode in ("user", "kernel"):
            for fault, vector in (("de", 0), ("ud", 6), ("gp", 13), ("pf", 14)):
                error = (6 if mode == "user" else 2) if fault == "pf" else 0
                if mode == "kernel" and fault == "gp":
                    error = 0x38
                validate_log(valid_log(mode, vector, error, 0x7000 if fault == "pf" else 0),
                             mode, fault)

    def test_missing_evidence_cannot_pass(self):
        log = valid_log()
        for line in log.splitlines():
            with self.subTest(line=line), self.assertRaises(ValueError):
                validate_log(log.replace(line + "\n", ""), "user", "ud")

    def test_wrong_vector_privilege_or_duplicate_fault_cannot_pass(self):
        log = valid_log()
        for changed in (log.replace("vector=6", "vector=13"),
                        log.replace("cpl=3", "cpl=0"),
                        log.replace("cs=0x33", "cs=0x8"),
                        log.replace("rsp=0x500000", "rsp=0x0"),
                        log.replace("error=0x0", "error=0x38"), log + log):
            with self.assertRaises(ValueError):
                validate_log(changed, "user", "ud")

    def test_kernel_recovery_or_a_returned_probe_is_not_a_halt(self):
        for extra in ("GENOS_READY", "KERNEL_EXCEPTION_PROBE_RETURNED", "EXCEPTION_FATAL_HALT"):
            with self.assertRaises(ValueError):
                validate_log(valid_log("kernel") + extra + "\n", "kernel", "ud")

    def test_fixture_requires_exactly_one_anchor(self):
        self.assertEqual(replace_once("before", "before", "after"), "after")
        for text in ("", "before before"):
            with self.assertRaises(ValueError):
                replace_once(text, "before", "after")

    def test_user_fixture_only_changes_probe_and_expected_result(self):
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            init = root / "userspace/init/src/main.rs"
            kernel = root / "kernel/src/userspace.rs"
            init.parent.mkdir(parents=True)
            kernel.parent.mkdir(parents=True)
            init.write_text("write_volatile(runtime::STACK_GUARD as *mut u64, token);\n")
            kernel.write_text("const FAULT_EXIT_CODE: u8 = 128 + 14;\n"
                              "faulting.fault_vector == 14\n"
                              "faulting.fault_error == 0x6\n"
                              "faulting.fault_address == paging::USER_STACK_GUARD\n")
            patch = patch_fixture(root, "user", "de")
            self.assertIn('"div rax"', init.read_text())
            self.assertIn("faulting.fault_vector == 0", kernel.read_text())
            self.assertNotIn("interrupts.rs", patch)
            self.assertNotIn("terminate_process_fault", patch)
            self.assertEqual(patch_fixture(root, "user", "pf"), "")


if __name__ == "__main__":
    unittest.main()

"""Host regressions for BSP evidence: missing/repeated guards must never pass."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from test_bsp import ENTRY, ENTRY_REJECTION, IDENTITY_SAMPLE, TABLE_REJECTION, patch_fixture, validate_log

TOPOLOGY = ("SMP_DISABLED policy=bsp-only active_cpus=1 initial_apic_id=0"
            " cpuid_max_logical_per_package=4")


def valid_log(case):
    lines = []
    if case != "non-bsp":
        lines += ["GenOS kernel entered"]
    if case in {"single", "quad", "repeat-init"}:
        lines += [TOPOLOGY, "GDT/TSS initialized", "IDT initialized"]
    if case in {"single", "quad"}:
        lines += ["GENOS_READY", "NORMAL_SHELL_READY"]
    else:
        lines += [f"BSP_PROBE_ARMED case={case}"]
        if case == "non-bsp":
            lines += ["BSP_PROBE_NON_BSP_INJECTED"]
        lines += ["BSP_PROBE_REJECTED"]
    return "\n".join(lines) + "\n"


class BspEvidenceTests(unittest.TestCase):
    def test_production_entry_admits_cpu_before_any_other_operation(self):
        main = (Path(__file__).resolve().parents[1] / "kernel/src/main.rs").read_text()
        self.assertEqual(main.count(ENTRY), 1)
        body = main.split(ENTRY, 1)[1].lstrip()
        self.assertTrue(body.startswith(ENTRY_REJECTION.lstrip()),
                        "CPU admission must precede UART writes and BootInfo access")

    def test_all_five_expected_outcomes(self):
        for case in ("single", "quad", "repeat-entry", "repeat-init", "non-bsp"):
            validate_log(valid_log(case), case)

    def test_each_required_line_is_necessary(self):
        for case in ("single", "quad", "repeat-entry", "repeat-init", "non-bsp"):
            log = valid_log(case)
            for line in log.splitlines():
                with self.subTest(case=case, missing=line), self.assertRaises(ValueError):
                    validate_log(log.replace(line + "\n", ""), case)

    def test_boot_reentry_and_false_topology_are_rejected(self):
        for changed in (valid_log("quad") + "GenOS kernel entered\n",
                        valid_log("quad") + TOPOLOGY + "\n",
                        valid_log("quad").replace("active_cpus=1", "active_cpus=4"),
                        valid_log("quad").replace("cpuid_max_logical_per_package=4",
                                                  "cpuid_max_logical_per_package=1")):
            with self.assertRaises(ValueError):
                validate_log(changed, "quad")

    def test_rejected_entry_cannot_initialize_or_continue(self):
        for case in ("repeat-entry", "repeat-init", "non-bsp"):
            for extra in ("GenOS kernel entered", "GDT/TSS initialized", "IDT initialized",
                          "BSP_PROBE_REJECTED", "GENOS_READY", "NORMAL_SHELL_READY",
                          "EXCEPTION_FRAME vector=14", "KERNEL PANIC", "BSP_PROBE_RETURNED"):
                with self.subTest(case=case, extra=extra), self.assertRaises(ValueError):
                    validate_log(valid_log(case) + extra + "\n", case)

    def test_fixture_preserves_guards_and_only_adds_injection_and_diagnostics(self):
        for case in ("single", "quad", "repeat-entry", "repeat-init", "non-bsp"):
            with TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "kernel/src").mkdir(parents=True)
                main = root / "kernel/src/main.rs"
                arch = root / "kernel/src/arch.rs"
                main.write_text(ENTRY + "\n" + ENTRY_REJECTION + "\n"
                                '    serial::println("GenOS kernel entered");\n'
                                "    arch::init();\n}\n")
                arch.write_text(TABLE_REJECTION + "\n" + IDENTITY_SAMPLE)
                patch = patch_fixture(root, case)
                if case in {"single", "quad"}:
                    self.assertEqual(patch, "")
                else:
                    self.assertIn("if !arch::claim_boot_cpu()", main.read_text())
                    self.assertIn("if !BOOT_CLAIM.begin_table_init(features, apic_base)", arch.read_text())
                    self.assertNotIn("boot_cpu.rs", patch)
                    self.assertNotIn("memory.rs", patch)
                    self.assertNotIn("userspace.rs", patch)


if __name__ == "__main__":
    unittest.main()

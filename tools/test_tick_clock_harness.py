"""Reject clock regressions, fabricated IRQ progress and incomplete evidence."""
from pathlib import Path
import tempfile
import unittest

from test_tick_clock import patch_fixture, validate_log

GOOD = "\n".join(("BOOT_MODE validation", "BOOT_MEMORY_MAP_VALIDATED", "IRQ_READY",
    "PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx",
    "TICK_CLOCK_ARMED fallback=3 hardware=0 if=0", "IRQ_HARDWARE_ON",
    "TICK_CLOCK_RESULT before=3 first=4 final=6 first_irqs=1 irqs=3 monotonic=1 fallback_stopped=1 if_enabled=1 if_disabled=1", ""))


class TickClockHarnessTests(unittest.TestCase):
    def test_complete_transition_passes(self):
        validate_log(GOOD)

    def test_missing_duplicate_embedded_reordered_and_resumed_records_fail(self):
        for line in GOOD.splitlines():
            for bad in (GOOD.replace(line + "\n", ""), GOOD + line + "\n", GOOD.replace(line, "forged " + line)):
                with self.subTest(line=line), self.assertRaises(ValueError):
                    validate_log(bad)
        for bad in ("\n".join(reversed(GOOD.splitlines())), GOOD + "continued\n", GOOD + "GENOS_READY\n"):
            with self.assertRaises(ValueError):
                validate_log(bad)

    def test_backward_frozen_fallback_or_wrong_if_never_pass(self):
        for old, new in (("first=4", "first=1"), ("first=4", "first=3"), ("final=6", "final=4"),
                         ("irqs=3", "irqs=0"), ("monotonic=1", "monotonic=0"),
                         ("fallback_stopped=1", "fallback_stopped=0"),
                         ("if_enabled=1", "if_enabled=0"), ("if_disabled=1", "if_disabled=0")):
            with self.subTest(old=old), self.assertRaises(ValueError):
                validate_log(GOOD.replace(old, new))

    def test_fixture_only_observes_production_clock_and_rejects_reinstall(self):
        source = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("kernel/src/main.rs", "kernel/src/interrupts.rs"):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text((source / name).read_text())
            original = (root / "kernel/src/interrupts.rs").read_text()
            fixture = patch_fixture(root)
            self.assertIn("TICK_CLOCK_RESULT", fixture)
            self.assertEqual((root / "kernel/src/interrupts.rs").read_text(), original)
            self.assertNotIn("tick_clock.rs", fixture)
            with self.assertRaises(ValueError):
                patch_fixture(root)


if __name__ == "__main__":
    unittest.main()

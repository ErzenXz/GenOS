"""Guard the frozen kernel-and-console progress denominator."""

import unittest
from pathlib import Path

import console_progress


ROOT = Path(__file__).resolve().parents[1]
ROADMAP = (ROOT / "ROADMAP.md").read_text()
BASELINE = (ROOT / "tools/console-progress-baseline.json").read_bytes()


class ConsoleProgressTests(unittest.TestCase):
    def test_original_denominator_and_initial_progress(self):
        result = console_progress.audit(ROADMAP, BASELINE)
        self.assertEqual((result["baseline_complete"], result["total"]), (26, 100))
        self.assertGreaterEqual(result["complete"], result["baseline_complete"])
        self.assertEqual(result["groups"]["Applications and terminal"]["total"], 19)

    def test_changed_criterion_cannot_preserve_the_percentage(self):
        changed = ROADMAP.replace("**F0.1:** resolve authorized", "**F0.1:** waive authorized", 1)
        self.assertNotEqual(changed, ROADMAP)
        with self.assertRaisesRegex(ValueError, "criteria were added, removed"):
            console_progress.audit(changed, BASELINE)

    def test_new_checkmark_changes_only_the_numerator(self):
        current = ROADMAP.replace("- [ ] **F0.1:**", "- [x] **F0.1:**", 1)
        self.assertNotEqual(current, ROADMAP)
        before = console_progress.audit(ROADMAP, BASELINE)
        after = console_progress.audit(current, BASELINE)
        self.assertEqual(after["complete"], before["complete"] + 1)
        self.assertEqual(after["total"], before["total"])

    def test_modified_baseline_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Frozen baseline changed"):
            console_progress.audit(ROADMAP, BASELINE + b"\n")


if __name__ == "__main__":
    unittest.main()

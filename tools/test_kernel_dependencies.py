import unittest
from pathlib import Path

import check_kernel_dependencies as dependency


class KernelDependencyTests(unittest.TestCase):
    def test_current_production_modules_have_no_forbidden_dependency(self):
        self.assertEqual(dependency.scan(), [])

    def test_storage_cannot_import_presentation_even_through_a_group(self):
        path = Path("kernel/src/vfs.rs")
        self.assertTrue(dependency.violations(path, "use kernel::{display::{FixedText}};"))
        self.assertTrue(dependency.violations(path, "crate::shell::run_terminal();"))

    def test_renderer_cannot_gain_mutable_or_direct_authority(self):
        path = Path("kernel/src/display/manager.rs")
        self.assertTrue(dependency.violations(path, "use crate::storage::PersistentFs;"))
        self.assertTrue(dependency.violations(path, "fn draw(vfs: &mut RamVfs) {}"))
        self.assertEqual(dependency.violations(path, "fn draw(vfs: &RamVfs) {}"), [])


if __name__ == "__main__":
    unittest.main()

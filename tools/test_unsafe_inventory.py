"""Regression tests for the source inventory, not a semantic Rust verifier."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

import check_unsafe as inventory


class UnsafeInventoryTests(unittest.TestCase):
    def sites(self, source):
        return inventory.inventory_source("src/example.rs", source)

    def test_comments_and_nested_comments_are_not_code(self):
        source = "// unsafe { asm!(); }\n/* unsafe fn a() { /* unsafe {} */ } */\nunsafe { work(); }"
        self.assertEqual([site["kind"] for site in self.sites(source)], ["unsafe_block"])
        self.assertEqual(self.sites(source)[0]["line"], 3)

    def test_string_byte_c_and_raw_literals_are_opaque(self):
        source = r'''let a = "unsafe fn ignored() { asm!(\"x\"); }";
        let b = b"unsafe impl Trait for Thing {}";
        let c = c"unsafe {}";
        let d = r##"unsafe { \" } asm!(); "# still literal"##;
        let e = br#"unsafe fn x() {}"#;
        let f = cr"unsafe { }";
        unsafe { asm!("nop"); }'''
        self.assertEqual([site["kind"] for site in self.sites(source)], ["unsafe_block", "asm_macro"])

    def test_char_literals_lifetimes_and_raw_identifiers(self):
        source = r'''fn f<'a>(a: &'a str) {
            let _ = '\''; let _ = '"'; let _ = '\u{7b}'; let _ = b'\x7d';
            let r#unsafe = 1; let r#asm = 2; unsafe { use_it(a); }
        }'''
        self.assertEqual([site["kind"] for site in self.sites(source)], ["unsafe_block"])

    def test_all_unsafe_forms_and_assembly_are_categorized(self):
        source = '''unsafe fn a() { unsafe { work() } }
        unsafe extern "C" fn b() {}
        unsafe impl Send for X {}
        unsafe trait T { unsafe fn f(); }
        unsafe extern "C" { fn f(); }
        #[unsafe(no_mangle)] fn c() {}
        core::arch::global_asm!("nop");
        core::arch::naked_asm!("ret");'''
        self.assertEqual([site["kind"] for site in self.sites(source)], [
            "unsafe_fn", "unsafe_block", "unsafe_fn", "unsafe_impl", "unsafe_trait",
            "unsafe_fn", "unsafe_extern", "unsafe_attribute", "global_asm_macro", "naked_asm_macro"])

    def test_construct_hash_includes_body_after_function_arguments(self):
        first = self.sites("unsafe fn f(a: &[u8]) -> [u8; 1] { work(a); }")
        second = self.sites("unsafe fn f(a: &[u8]) -> [u8; 1] { changed(a); }")
        self.assertNotEqual(first[0]["construct_sha256"], second[0]["construct_sha256"])

    def test_unterminated_lexical_regions_fail_closed(self):
        for source in ['/* unsafe {}', 'let x = r##"unsafe {}"#;', 'let x = "unsafe {};', 'unsafe {']:
            with self.subTest(source=source), self.assertRaises(ValueError):
                self.sites(source)

    def test_inventory_exclusions_assembly_and_order_are_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "z.rs").write_text("unsafe { z(); }")
            (root / "a.S").write_text(".text\nret\n")
            (root / "nested" / "target").mkdir(parents=True)
            (root / "nested" / "target" / "ignore.rs").write_text("unsafe { ignored(); }")
            result = inventory.build_inventory(root)
            self.assertEqual([entry["path"] for entry in result["files"]], ["a.S", "z.rs"])
            self.assertEqual(result["counts"], {"assembly_file": 1, "unsafe_block": 1})
            self.assertIn("target", result["scope"]["excluded_directory_names"])
            self.assertEqual(result, inventory.build_inventory(root))

    def test_check_detects_removed_site_and_same_count_context_change(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            root = Path(directory)
            source = root / "main.rs"
            source.write_text("// SAFETY: one owner\nunsafe { old(); }\n")
            args = ["--root", directory]
            self.assertEqual(inventory.main(args + ["--write-baseline"]), 0)
            baseline = root / "docs" / "unsafe-inventory.json"
            original_baseline = baseline.read_bytes()
            self.assertEqual(inventory.main(args + ["--check"]), 0)
            source.write_text("// SAFETY: different obligation\nunsafe { old(); }\n")
            self.assertEqual(inventory.main(args + ["--check"]), 1)
            self.assertEqual(baseline.read_bytes(), original_baseline)
            source.write_text("fn safe() {}\n")
            self.assertEqual(inventory.main(args + ["--diff"]), 1)
            self.assertEqual(baseline.read_bytes(), original_baseline)

    def test_missing_malformed_and_scope_reduced_baseline_fail(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            root = Path(directory)
            (root / "main.rs").write_text("unsafe { x() }")
            args = ["--root", directory]
            self.assertEqual(inventory.main(args), 1)
            self.assertEqual(inventory.main(args + ["--write-baseline"]), 0)
            baseline = root / "docs" / "unsafe-inventory.json"
            parsed = json.loads(baseline.read_text())
            parsed["sites"][0].pop("context")
            baseline.write_text(json.dumps(parsed))
            self.assertEqual(inventory.main(args + ["--check"]), 1)
            baseline.write_text("[]")
            self.assertEqual(inventory.main(args + ["--check"]), 2)
            baseline.write_text("invalid JSON")
            self.assertEqual(inventory.main(args + ["--check"]), 2)

    def test_source_file_hash_covers_distant_caller_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "main.rs"
            source.write_text("unsafe { x(); }\n" + "\n" * 30 + "fn caller() { first(); }\n")
            before = inventory.build_inventory(root)
            source.write_text("unsafe { x(); }\n" + "\n" * 30 + "fn caller() { second(); }\n")
            after = inventory.build_inventory(root)
            self.assertEqual(before["sites"], after["sites"])
            self.assertNotEqual(before["files"], after["files"])

    def test_source_symlink_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "real.rs").write_text("unsafe { x(); }")
            (root / "alias.rs").symlink_to(root / "real.rs")
            with self.assertRaisesRegex(ValueError, "symlinks"):
                inventory.build_inventory(root)


if __name__ == "__main__":
    unittest.main()

"""Host-only tests for reference identity and shared probe arguments."""
from pathlib import Path
import unittest

from reference_vm import load_profile, mismatches, parse_profile, profile_path, qemu_args


class ReferenceProfileTests(unittest.TestCase):
    def test_each_profile_field_is_required_once(self):
        source = profile_path().read_text()
        for line in source.splitlines():
            if not line or line.startswith("#"):
                continue
            for modified in (source.replace(line + "\n", ""), source + line + "\n"):
                with self.subTest(line=line), self.assertRaises(ValueError):
                    parse_profile(modified)
        for suffix in ("unknown=1\n", "missing separator\n", "cpu=\n"):
            with self.assertRaises(ValueError):
                parse_profile(source + suffix)

    def test_mismatched_tool_or_firmware_is_not_reference_evidence(self):
        profile = load_profile()
        rust = "rustc metadata\nrelease: " + profile["rust_release"]
        qemu = profile["qemu_version"] + "\nCopyright"
        digest = profile["firmware_sha256"]
        self.assertEqual(mismatches(profile, rust, qemu, digest), [])
        self.assertEqual(mismatches(profile, "unavailable", qemu, digest), ["rust_release"])
        self.assertEqual(mismatches(profile, rust, qemu.replace("11.1.1", "11.1.10"), digest), ["qemu_version"])
        self.assertEqual(mismatches(profile, rust, qemu, "0" * 64), ["firmware_sha256"])

    def test_bsp_topology_is_the_only_profile_variant(self):
        single = qemu_args()
        quad = qemu_args(4)
        self.assertEqual(single[:-1], quad[:-1])
        self.assertEqual(quad[-1], "4,sockets=1,cores=4,threads=1")
        self.assertEqual(single[single.index("-machine") + 1], "pc-q35-8.2")
        self.assertEqual(single[single.index("-cpu") + 1], "qemu64-v1,smep=on,smap=on")
        with self.assertRaises(ValueError):
            qemu_args(2)


if __name__ == "__main__":
    unittest.main()

"""Host regressions for staged-kernel mutation and exact loader rejection."""
import struct
import unittest

from test_kernel_elf import CASES, mutate_elf, validate_log


def valid_elf():
    data = bytearray(0x3000)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HH", data, 54, 56, 2)
    for index, flags in enumerate((5, 4)):
        address = 0x2000000 + index * 4096
        struct.pack_into("<IIQQQQQQ", data, 64 + index * 56,
                         1, flags, (index + 1) * 4096, address, address, 16, 4096, 4096)
    return bytes(data)


def valid_log(case):
    return ("GenOS UEFI loader v1\nLoading kernel ELF\nKERNEL_ELF_REJECTED reason="
            + CASES[case] + "\nGenOS boot failed: LOAD_ERROR\n")


class KernelElfHarnessTests(unittest.TestCase):
    def test_every_case_changes_only_its_recorded_bytes_or_truncation(self):
        original = valid_elf()
        for case in CASES:
            mutated, record = mutate_elf(original, case)
            self.assertNotEqual(mutated, original)
            self.assertEqual(record["expected_error"], CASES[case])
            if case == "truncated-header":
                self.assertEqual(mutated, original[:4])
            else:
                reconstructed = bytearray(original)
                for edit in record["edits"]:
                    before = bytes.fromhex(edit["before_hex"])
                    after = bytes.fromhex(edit["after_hex"])
                    offset = edit["offset"]
                    self.assertEqual(reconstructed[offset:offset + len(before)], before)
                    reconstructed[offset:offset + len(after)] = after
                self.assertEqual(mutated, reconstructed)
        self.assertEqual(original, valid_elf())
        for invalid in (b"", b"\x7fELF", b"x" * 128):
            with self.assertRaises(ValueError):
                mutate_elf(invalid, "overlap")

    def test_each_exact_ordered_rejection_record_is_required_once(self):
        for case in CASES:
            log = valid_log(case)
            validate_log(log, case)
            for line in log.splitlines():
                for wrong in (log.replace(line + "\n", ""), log + line + "\n",
                              log.replace(line, "forged " + line)):
                    with self.subTest(case=case, line=line), self.assertRaises(ValueError):
                        validate_log(wrong, case)
            with self.assertRaises(ValueError):
                validate_log("\n".join(reversed(log.splitlines())), case)

    def test_wrong_reason_kernel_entry_or_partial_continuation_is_rejected(self):
        for extra in ("GenOS kernel entered", "BOOT_MEMORY_MAP_VALIDATED",
                      "Loading initrd", "BOOTLOADER_PANIC", "GENOS_READY",
                      "KERNEL_ELF_REJECTED reason=Permissions"):
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                validate_log(valid_log("truncated-header") + extra + "\n", "truncated-header")
        with self.assertRaises(ValueError):
            validate_log(valid_log("program-range"), "overlap")


if __name__ == "__main__":
    unittest.main()

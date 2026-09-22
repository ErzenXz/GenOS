import unittest

from test_boot_memory import CASES, validate_log


class BootMemoryEvidenceTests(unittest.TestCase):
    def test_exact_phase_and_rejection(self):
        for case in CASES:
            log = ("BOOT_MEMORY_MAP_REJECTED\n" if case == "loader-version"
                   else "GenOS kernel entered\nBOOT_INFO_REJECTED\n")
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


if __name__ == "__main__":
    unittest.main()

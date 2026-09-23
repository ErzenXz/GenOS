import unittest

from test_startup_dependencies import StartupProof


def feed_ready(proof):
    for line in ("BOOT_MODE normal", "PERSISTENT_STORAGE_UNAVAILABLE", "GENOS_READY"):
        assert proof.observe(line) is None
    return proof.observe("NORMAL_SHELL_READY")


class StartupDependencyHarnessTests(unittest.TestCase):
    def test_fresh_ordered_console_commands_are_required(self):
        proof = StartupProof("nonce")
        self.assertEqual(feed_ready(proof), b"echo nonce\r")
        with self.assertRaisesRegex(ValueError, "reply before"):
            proof.observe("nonce")
        proof.observe("genos> echo nonce")
        self.assertEqual(proof.observe("nonce"), b"net\r")
        proof.observe("genos> net")
        self.assertEqual(proof.observe("network unavailable"), b"mem\r")
        proof.observe("genos> mem")
        with self.assertRaisesRegex(ValueError, "reply before"):
            proof.observe("consistent=yes")
        proof.observe("frames_total=100 live=25 free=75")
        self.assertIsNone(proof.observe("consistent=yes"))
        self.assertTrue(proof.complete)

    def test_false_or_repeated_startup_records_fail(self):
        for log in (
            ("GENOS_READY",),
            ("BOOT_MODE normal", "GENOS_READY"),
            ("BOOT_MODE validation",),
            ("BOOT_MODE normal", "PERSISTENT_STORAGE_UNAVAILABLE",
             "GENOS_READY", "GENOS_READY"),
            ("BOOT_MODE normal", "PERSISTENT_STORAGE_UNAVAILABLE",
             "GENOS_READY", "USER_CONSOLE_WRITE pid=1 text=nonce"),
        ):
            proof = StartupProof("nonce")
            with self.assertRaises(ValueError):
                for line in log:
                    proof.observe(line)


if __name__ == "__main__":
    unittest.main()

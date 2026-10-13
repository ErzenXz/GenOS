"""Host-only admission and QMP protocol checks for CPU qualification."""
import json
from pathlib import Path
import unittest
from unittest.mock import MagicMock, call, patch

from cpu_backend import preflight, qmp_command, verify_accelerator


CPUINFO = "vendor_id : GenuineIntel\nmodel name : test CPU\nflags : sse sse2 smep smap\n\nvendor_id : ignored\n"


class BackendTests(unittest.TestCase):
    def test_tcg_requires_no_hardware_access(self):
        with patch("cpu_backend.os.open", side_effect=AssertionError("unexpected device access")):
            self.assertEqual(preflight("tcg"), {"cpu_backend": "tcg"})
        with self.assertRaises(ValueError):
            preflight("auto")

    def test_kvm_rejects_unsupported_platform_before_access(self):
        for system, machine in (("Darwin", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(system=system, machine=machine), \
                    patch("cpu_backend.platform.system", return_value=system), \
                    patch("cpu_backend.platform.machine", return_value=machine), \
                    patch("cpu_backend.os.open") as opened:
                with self.assertRaisesRegex(RuntimeError, "Linux x86_64"):
                    preflight("kvm")
                opened.assert_not_called()

    def run_preflight(self, ioctl_results, open_error=None):
        with patch("cpu_backend.platform.system", return_value="Linux"), \
                patch("cpu_backend.platform.machine", return_value="x86_64"), \
                patch("cpu_backend.platform.uname") as uname, \
                patch("cpu_backend.Path.read_text", return_value=CPUINFO), \
                patch("cpu_backend.os.open", return_value=21, side_effect=open_error), \
                patch("fcntl.ioctl", side_effect=ioctl_results) as ioctl, \
                patch("cpu_backend.os.close") as close:
            uname.return_value._asdict.return_value = {"release": "test-kernel"}
            try:
                return preflight("kvm"), ioctl, close
            except Exception:
                self.last_ioctl, self.last_close = ioctl, close
                raise

    def test_kvm_requires_api_and_vm_creation_and_retains_identity(self):
        result, ioctl, close = self.run_preflight([12, 22])
        self.assertEqual(result["cpu_backend"], "kvm")
        self.assertEqual(result["kvm_api_version"], 12)
        self.assertEqual(result["host_cpu"]["vendor_id"], "GenuineIntel")
        self.assertEqual(result["host_cpu"]["model name"], "test CPU")
        self.assertIn("release", result["host_uname"])
        self.assertEqual(ioctl.call_args_list, [call(21, 0xAE00, 0), call(21, 0xAE01, 0)])
        self.assertEqual(close.call_args_list, [call(22), call(21)])

    def test_kvm_fails_closed_and_closes_open_descriptors(self):
        for responses, error in (([11], RuntimeError), ([12, OSError("no VM")], OSError),
                                 ([OSError("no API")], OSError), ([12, -1], RuntimeError)):
            with self.subTest(responses=responses), self.assertRaises(error):
                self.run_preflight(responses)
            self.last_close.assert_called_once_with(21)
        with self.assertRaises(PermissionError):
            self.run_preflight([], PermissionError("device denied"))
        self.last_ioctl.assert_not_called()
        self.last_close.assert_not_called()

    def test_kvm_rejects_missing_host_identity(self):
        with patch("cpu_backend.platform.system", return_value="Linux"), \
                patch("cpu_backend.platform.machine", return_value="x86_64"), \
                patch("cpu_backend.platform.uname") as uname, \
                patch("cpu_backend.Path.read_text", return_value="flags : sse\n"), \
                patch("cpu_backend.os.open") as opened:
            uname.return_value._asdict.return_value = {"release": "test-kernel"}
            with self.assertRaisesRegex(RuntimeError, "host CPU identity"):
                preflight("kvm")
            opened.assert_not_called()

    def test_accelerator_readback_requires_exact_booleans(self):
        verify_accelerator("tcg", {"enabled": False, "present": True})
        verify_accelerator("kvm", {"enabled": True, "present": True})
        for backend, result in (("kvm", {"enabled": False, "present": True}),
                                ("kvm", {"enabled": True, "present": False}),
                                ("kvm", {"enabled": 1, "present": True}),
                                ("tcg", {"enabled": 0}), ("tcg", {"enabled": True}),
                                ("tcg", {}), ("kvm", None)):
            with self.subTest(backend=backend, result=result), self.assertRaises(RuntimeError):
                verify_accelerator(backend, result)
        with self.assertRaises(ValueError):
            verify_accelerator("auto", {})


class QmpTests(unittest.TestCase):
    def exchange(self, responses, request=None):
        wire = MagicMock()
        wire.__enter__.return_value = wire
        wire.readline.side_effect = [json.dumps(item).encode() + b"\n" for item in responses] + [b""]
        connection = MagicMock()
        connection.__enter__.return_value = connection
        connection.makefile.return_value = wire
        transcript = []
        with patch("cpu_backend.socket.socket", return_value=connection):
            try:
                result = qmp_command(Path("/tmp/test.qmp"), request or {"execute": "query-kvm"}, transcript)
            finally:
                self.transcript, self.wire, self.connection = transcript, wire, connection
        return result

    def test_greeting_capabilities_events_and_matching_result_are_retained(self):
        result = {"present": True, "enabled": True}
        self.assertEqual(self.exchange([
            {"QMP": {"version": {}}},
            {"event": "STOP"},
            {"return": {}, "id": "genos-cpu-capabilities"},
            {"return": result, "id": 7},
        ], {"execute": "query-kvm", "id": 7}), result)
        self.assertEqual(len(self.transcript), 6)
        self.assertEqual([item["message"]["execute"] for item in self.transcript
                          if item["direction"] == "sent"], ["qmp_capabilities", "query-kvm"])
        self.connection.connect.assert_called_once_with("/tmp/test.qmp")
        self.connection.settimeout.assert_called_once_with(10)
        self.assertEqual(self.wire.flush.call_count, 2)

    def test_rejects_missing_greeting_error_wrong_id_and_absent_return(self):
        greeting = {"QMP": {"version": {}}}
        cap = {"return": {}, "id": "genos-cpu-capabilities"}
        cases = [[], [{}], [greeting, {"error": {"class": "Nope"}, "id": cap["id"]}],
                 [greeting, cap, {"error": {"class": "Nope"}, "id": "genos-cpu-request"}],
                 [greeting, cap, {"return": {}, "id": "wrong"}],
                 [greeting, cap, {"id": "genos-cpu-request"}],
                 [greeting, cap, []]]
        for responses in cases:
            with self.subTest(responses=responses), self.assertRaises(RuntimeError):
                self.exchange(responses)

    def test_invalid_request_is_rejected_before_connecting(self):
        for request in ({}, {"execute": 1}, {"execute": "query-kvm", "id": "genos-cpu-capabilities"}):
            with patch("cpu_backend.socket.socket") as opened, self.assertRaises(ValueError):
                qmp_command(Path("unused"), request, [])
            opened.assert_not_called()


if __name__ == "__main__":
    unittest.main()

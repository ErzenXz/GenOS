"""Host regressions for staged-kernel rejection and its QMP shutdown evidence."""
import io
import json
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
import unittest
from unittest import mock

from test_kernel_elf import (CAPABILITIES_ID, CASES, START_ID, QmpMonitor, boot_with_qmp,
                             collect_shutdown, connect_qmp, mutate_elf, validate_log, validate_qmp)


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


def valid_qmp():
    return [{"QMP": {"version": {}, "capabilities": []}},
            {"return": {}, "id": CAPABILITIES_ID},
            {"event": "RESUME"}, {"return": {}, "id": START_ID},
            {"event": "SHUTDOWN", "data": {"guest": True, "reason": "guest-shutdown"}}]


def qmp_wire(messages):
    return b"".join((json.dumps(message) + "\r\n").encode() for message in messages)


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


class QmpShutdownTests(unittest.TestCase):
    def monitor_for(self, wire):
        connection, peer = socket.socketpair()
        self.addCleanup(connection.close)
        self.addCleanup(peer.close)
        peer.sendall(wire)
        peer.shutdown(socket.SHUT_WR)
        received, sent = io.BytesIO(), io.BytesIO()
        return QmpMonitor(connection, received, sent), peer, received, sent

    def test_requires_explicit_guest_shutdown_after_execution(self):
        messages = valid_qmp()
        self.assertEqual(validate_qmp(messages), {"guest": True, "reason": "guest-shutdown"})
        # QMP may emit RESUME on either side of the cont acknowledgement.
        messages[2], messages[3] = messages[3], messages[2]
        validate_qmp(messages)
        for data in ({}, {"guest": True}, {"reason": "guest-shutdown"}, None,
                     {"guest": False, "reason": "guest-shutdown"},
                     {"guest": 1, "reason": "guest-shutdown"},
                     {"guest": "true", "reason": "guest-shutdown"}):
            bad = valid_qmp()
            bad[-1]["data"] = data
            with self.subTest(data=data), self.assertRaises(ValueError):
                validate_qmp(bad)
        for reason in ("guest-reset", "guest-panic", "host-error", "host-qmp-quit",
                       "host-signal", "host-ui", "none", "unknown"):
            bad = valid_qmp()
            bad[-1]["data"]["reason"] = reason
            with self.subTest(reason=reason), self.assertRaises(ValueError):
                validate_qmp(bad)

    def test_missing_duplicate_reordered_or_fault_events_are_rejected(self):
        original = valid_qmp()
        for index in range(len(original)):
            for bad in (original[:index] + original[index + 1:], original + [original[index]]):
                with self.subTest(index=index), self.assertRaises(ValueError):
                    validate_qmp(bad)
        for order in ((0, 2, 1, 3, 4), (0, 1, 4, 2, 3), (0, 3, 1, 2, 4)):
            with self.subTest(order=order), self.assertRaises(ValueError):
                validate_qmp([original[index] for index in order])
        for extra in ({"event": "RESET"}, {"event": "GUEST_PANICKED"}, {"event": "WATCHDOG"},
                      {"error": {"class": "GenericError"}}, {}, [],
                      {"event": "RESUME", "return": {}, "id": START_ID}):
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                validate_qmp(original + [extra])
        bad = valid_qmp()
        bad[1]["return"] = {"unexpected": True}
        with self.assertRaises(ValueError):
            validate_qmp(bad)

    def test_negotiation_commands_and_raw_wire_are_retained(self):
        wire = qmp_wire(valid_qmp())
        monitor, peer, received, sent = self.monitor_for(wire)
        monitor.start(time.monotonic() + 1)
        process = mock.Mock()
        process.poll.return_value = 0
        collect_shutdown(monitor, process, time.monotonic() + 1)
        commands = [json.loads(line) for line in sent.getvalue().splitlines()]
        self.assertEqual(commands, [{"execute": "qmp_capabilities", "id": CAPABILITIES_ID},
                                    {"execute": "cont", "id": START_ID}])
        peer.settimeout(1)
        self.assertEqual(peer.recv(4096), sent.getvalue())
        self.assertEqual(received.getvalue(), wire)
        self.assertEqual(monitor.messages, valid_qmp())

    def test_fragmented_messages_are_reassembled_without_losing_evidence(self):
        wire = qmp_wire(valid_qmp())
        connection = mock.Mock()
        connection.recv.side_effect = [bytes([byte]) for byte in wire] + [b""]
        received = io.BytesIO()
        monitor = QmpMonitor(connection, received, io.BytesIO())
        for expected in valid_qmp():
            self.assertEqual(monitor.read(time.monotonic() + 1), expected)
        self.assertIsNone(monitor.read(time.monotonic() + 1))
        self.assertEqual(received.getvalue(), wire)

    def test_eof_can_precede_successful_process_exit(self):
        monitor, _, _, _ = self.monitor_for(qmp_wire(valid_qmp()))
        monitor.start(time.monotonic() + 1)
        process = mock.Mock()
        process.poll.return_value = None
        process.wait.return_value = 0
        self.assertEqual(collect_shutdown(monitor, process, time.monotonic() + 1),
                         {"guest": True, "reason": "guest-shutdown"})
        process.wait.assert_called_once()
        self.assertGreater(process.wait.call_args.kwargs["timeout"], 0)
        self.assertLessEqual(process.wait.call_args.kwargs["timeout"], 1)

    def test_disconnected_live_process_or_abnormal_exit_is_rejected(self):
        for status in (None, -15, 1):
            monitor, _, received, _ = self.monitor_for(qmp_wire(valid_qmp()))
            monitor.start(time.monotonic() + 1)
            process = mock.Mock()
            process.poll.return_value = status
            process.wait.side_effect = subprocess.TimeoutExpired("qemu", 1)
            with self.subTest(status=status), self.assertRaises(RuntimeError):
                collect_shutdown(monitor, process, time.monotonic() + 1)
            self.assertEqual(received.getvalue(), qmp_wire(valid_qmp()))

    def test_exit_zero_with_reset_or_no_shutdown_is_rejected(self):
        reset = valid_qmp()
        reset[-1]["data"]["reason"] = "guest-reset"
        for messages in (reset, valid_qmp()[:-1]):
            monitor, _, _, _ = self.monitor_for(qmp_wire(messages))
            monitor.start(time.monotonic() + 1)
            process = mock.Mock()
            process.poll.return_value = 0
            with self.assertRaises(ValueError):
                collect_shutdown(monitor, process, time.monotonic() + 1)

    def test_failed_negotiation_cannot_start_the_guest(self):
        for tail in ([], [{"return": {}, "id": "wrong"}], [{"error": {"class": "GenericError"}}]):
            monitor, _, _, sent = self.monitor_for(qmp_wire(valid_qmp()[:1] + tail))
            with self.subTest(tail=tail), self.assertRaises((RuntimeError, ValueError)):
                monitor.start(time.monotonic() + 1)
            self.assertNotIn(b'"cont"', sent.getvalue())

    def test_malformed_and_truncated_wire_is_rejected_and_retained(self):
        for wire in (b"[]\n", b"garbage\n", b'{"event":\n', b'{"event": "SHUTDOWN"}'):
            monitor, _, received, _ = self.monitor_for(wire)
            with self.subTest(wire=wire), self.assertRaises(ValueError):
                monitor.read(time.monotonic() + 1)
            self.assertEqual(received.getvalue(), wire)

    def test_protocol_memory_and_count_budgets_fail_closed(self):
        for limit, size, wire in (("MAX_QMP_MESSAGE_BYTES", 16, b'{"x": "' + b"x" * 20 + b'"}\n'),
                                  ("MAX_QMP_TOTAL_BYTES", 16, qmp_wire([{}, {}, {}, {}, {}])),
                                  ("MAX_QMP_MESSAGES", 2, qmp_wire([{}, {}, {}]))):
            monitor, _, received, _ = self.monitor_for(wire)
            with self.subTest(limit=limit), mock.patch("test_kernel_elf." + limit, size):
                with self.assertRaises(ValueError):
                    while monitor.read(time.monotonic() + 1) is not None:
                        pass
            self.assertEqual(received.getvalue(), wire)

    def test_silent_connection_and_live_guest_have_deadlines(self):
        connection, peer = socket.socketpair()
        self.addCleanup(connection.close)
        self.addCleanup(peer.close)
        monitor = QmpMonitor(connection, io.BytesIO(), io.BytesIO())
        with self.assertRaises(TimeoutError):
            monitor.read(time.monotonic() + 0.02)
        process = mock.Mock()
        process.poll.return_value = None
        with self.assertRaises(TimeoutError):
            collect_shutdown(monitor, process, time.monotonic() + 0.02)
        process.wait.assert_not_called()

    def test_connection_deadline_and_early_qemu_exit(self):
        with tempfile.TemporaryDirectory(prefix="genos-qmp-test-", dir="/tmp") as directory:
            process = mock.Mock()
            process.poll.return_value = None
            with self.assertRaises(TimeoutError):
                connect_qmp(Path(directory) / "missing", process, time.monotonic() + 0.02)
            process.poll.return_value = 0
            with self.assertRaises(RuntimeError):
                connect_qmp(Path(directory) / "missing", process, time.monotonic() + 1)

    def test_boot_retains_wire_and_requires_guest_shutdown_even_with_valid_serial(self):
        for reason in ("guest-shutdown", "guest-reset"):
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as directory:
                evidence = Path(directory)
                serial = evidence / "serial.log"
                serial.write_text(valid_log("overlap"))
                messages = valid_qmp()
                messages[-1]["data"]["reason"] = reason
                monitor, _, _, _ = self.monitor_for(qmp_wire(messages))
                process = mock.Mock()
                process.poll.return_value = 0
                process.wait.return_value = 0
                with (mock.patch("test_kernel_elf.qemu_args", return_value=["qemu-test"]),
                      mock.patch("test_kernel_elf.firmware_path", return_value="firmware.fd"),
                      mock.patch("test_kernel_elf.load_profile", return_value={"boot_drive": "raw"}),
                      mock.patch("test_kernel_elf.subprocess.Popen", return_value=process),
                      mock.patch("test_kernel_elf.connect_qmp", return_value=monitor.connection)):
                    if reason == "guest-shutdown":
                        boot_with_qmp(evidence, evidence, "overlap", 1, serial, Path("/tmp/unused"))
                    else:
                        with self.assertRaises(ValueError):
                            boot_with_qmp(evidence, evidence, "overlap", 1, serial, Path("/tmp/unused"))
                self.assertEqual((evidence / "qmp.log").read_bytes(), qmp_wire(messages))
                commands = [json.loads(line) for line in (evidence / "qmp-commands.log").read_text().splitlines()]
                self.assertEqual([command["execute"] for command in commands], ["qmp_capabilities", "cont"])
                command = json.loads((evidence / "qemu-command.json").read_text())
                self.assertIn("-S", command)
                self.assertEqual(command[command.index("-qmp") + 1], "unix:/tmp/unused,server=on,wait=off")
                process.terminate.assert_not_called()


if __name__ == "__main__":
    unittest.main()

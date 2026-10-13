"""Negative evidence, provenance and terminal-state checks for firmware faults."""
import copy
import hashlib
import io
import json
from pathlib import Path
import tarfile
from tempfile import TemporaryDirectory
import unittest

from test_firmware_lifecycle import CASES, validate_log, validate_qmp, verify_package, STATUS_ID
from test_kernel_elf import CAPABILITIES_ID, START_ID, MAX_QMP_MESSAGES


def map_record(call, status="success", size=384, capacity=768):
    return f"FIRMWARE_MAP_RESULT call={call} status={status} size={size} capacity={capacity}\n"


def log_for(case):
    mode, stage, pages, outcome = CASES[case]
    if stage:
        log = f"FIRMWARE_ALLOCATION_ARMED stage={stage}\nFIRMWARE_ALLOCATION_DENIED stage={stage} status=out-of-resources\n"
        log += "BOOTLOADER_PANIC\n" if stage == 4 else "GenOS boot failed: OUT_OF_RESOURCES\n"
    elif pages:
        log = map_record(1) + f"FIRMWARE_MAP_GROWN pages={pages} before=384 capacity=768 stride=48\n"
        if pages == 1:
            log += map_record(2, size=480) + "FIRMWARE_EXIT_RESULT attempt=1 status=success\n"
        else:
            log += map_record(2,"buffer-too-small",1344) + map_record(3,"buffer-too-small",1344)
    else:
        log = map_record(1) + "FIRMWARE_EXIT_RESULT attempt=1 status=invalid-parameter\n"
        log += map_record(2) + f"FIRMWARE_EXIT_RESULT attempt=2 status={'success' if mode == 1 else 'invalid-parameter'}\n"
    if outcome == "running":
        log += "GenOS kernel entered\nBOOT_MEMORY_MAP_VALIDATED\nGENOS_READY\n"
    return log


def qmp_for(outcome):
    rows = [{"QMP":{}}, {"return":{},"id":CAPABILITIES_ID}, {"event":"RESUME"}, {"return":{},"id":START_ID}]
    if outcome in ("running", "panic"):
        rows += [{"id":STATUS_ID,"return":{"status":"running","running":True}}]
    else:
        rows += [{"event":"SHUTDOWN","data":{"guest":True,"reason":outcome}}]
    return rows


class FirmwareEvidenceTests(unittest.TestCase):
    def test_each_case_has_exact_serial_and_terminal_evidence(self):
        for case, (_,_,_,outcome) in CASES.items():
            validate_log(log_for(case),case)
            validate_qmp(qmp_for(outcome),outcome)

    def test_every_omitted_duplicated_prefixed_or_reordered_record_fails(self):
        for case in CASES:
            good=log_for(case)
            records=good.splitlines(True)
            for index,line in enumerate(records):
                for bad in ["".join(records[:index]+records[index+1:]), good+line,
                            good.replace(line,"prefix "+line), "".join(reversed(records))]:
                    with self.subTest(case=case,line=line), self.assertRaises(ValueError):
                        validate_log(bad,case)
            with self.assertRaises(ValueError):validate_log(good.rstrip("\n"),case)

    def test_wrong_error_extent_call_identity_stage_and_success_are_denied(self):
        for case in CASES:
            good=log_for(case)
            mutations = [good+"KERNEL PANIC\n",good+"EXCEPTION_FRAME\n"]
            if "FIRMWARE_MAP_RESULT" in good:
                mutations += [good.replace("call=2","call=1"),good.replace("capacity=768","capacity=1")]
            if "invalid-parameter" in good: mutations += [good.replace("invalid-parameter","success")]
            if "pages=" in good: mutations += [good.replace("pages=", "pages=9"),good.replace("stride=48","stride=1")]
            if "ARMED" in good: mutations += [good.replace("ARMED stage=","ARMED stage=9")]
            if CASES[case][3] != "running": mutations += [good+"GenOS kernel entered\n"]
            for bad in mutations:
                with self.subTest(case=case), self.assertRaises(ValueError): validate_log(bad,case)

    def test_qmp_unknown_duplicate_wrong_phase_wrong_reason_and_paused_guest_fail(self):
        for outcome in ("running","panic","guest-reset","guest-shutdown"):
            good=qmp_for(outcome)
            for i in range(len(good)):
                for bad in [good[:i]+good[i+1:],good+[good[i]]]:
                    with self.assertRaises(ValueError):validate_qmp(bad,outcome)
            for event in ["RESET","STOP","WATCHDOG","GUEST_PANICKED","OTHER"]:
                with self.assertRaises(ValueError):validate_qmp(good+[{"event":event}],outcome)
            bad=copy.deepcopy(good)
            if outcome in ("running","panic"):
                bad[-1]["return"]["running"]=False
            else:
                bad[-1]["data"]["reason"]="host-error"
            with self.assertRaises(ValueError):validate_qmp(bad,outcome)
            with self.assertRaises(ValueError):validate_qmp([None],outcome)

    def test_normal_firmware_rtc_initialization_is_bounded_and_not_terminal_evidence(self):
        for outcome in ("running","panic","guest-reset","guest-shutdown"):
            good=qmp_for(outcome)
            event={"event":"RTC_CHANGE","data":{"offset":0,"qom-path":"/machine/unattached/rtc"}}
            with_clock=good[:-1]+[event]+good[-1:]
            validate_qmp(with_clock,outcome)
            validate_qmp(good[:-1]+[event,event]+good[-1:],outcome)
            for bad in [good[:-1]+[event]*MAX_QMP_MESSAGES+good[-1:], [event]+good,
                        good+[event],good[:-1]+[{"event":"RTC_CHANGE","data":{}}]+good[-1:]]:
                with self.assertRaises(ValueError):validate_qmp(bad,outcome)

    def test_retained_qemu82_two_rtc_events_do_not_substitute_for_terminal_proof(self):
        path=Path(__file__).resolve().parent/"fixtures/firmware-qemu82-rtc.jsonl"
        rows=[json.loads(line) for line in path.read_text().splitlines()]
        validate_qmp(rows,"running")
        with self.assertRaises(ValueError):validate_qmp(rows[:-1],"running")
        with self.assertRaises(ValueError):validate_qmp(rows+[rows[-1]],"running")


class FirmwareSourceTests(unittest.TestCase):
    def test_only_locked_original_files_can_reach_fixture_patching(self):
        with TemporaryDirectory() as temporary:
            root=Path(temporary); package=root/"vendor";package.mkdir()
            (package/"src").mkdir();(package/"src/boot.rs").write_bytes(b"original")
            archive=root/"uefi.crate"
            with tarfile.open(archive,"w:gz") as t:
                info=tarfile.TarInfo("uefi-0.36.1/src/boot.rs");info.size=8;t.addfile(info,io.BytesIO(b"original"))
            expected=hashlib.sha256(archive.read_bytes()).hexdigest()
            checksums={"package":expected,"files":{"src/boot.rs":hashlib.sha256(b"original").hexdigest()}}
            receipt=package/".cargo-checksum.json";receipt.write_text(json.dumps(checksums))
            verify_package(package,archive,expected)
            (package/"src/extra.rs").write_text("unlocked")
            with self.assertRaises(ValueError):verify_package(package,archive,expected)
            (package/"src/extra.rs").unlink()
            (package/"src/boot.rs").write_bytes(b"tampered")
            checksums["files"]["src/boot.rs"]=hashlib.sha256(b"tampered").hexdigest()
            receipt.write_text(json.dumps(checksums))
            with self.assertRaises(ValueError):verify_package(package,archive,expected)
            with self.assertRaises(ValueError):verify_package(package,archive,"0"*64)


if __name__ == "__main__":unittest.main()

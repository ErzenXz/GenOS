#!/usr/bin/env python3
"""Observe fallback-to-PIT clock transition through the production timer APIs."""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import time

from reference_vm import environment
from test_cpu_paging import boot
from test_exception_entry import firmware_path, replace_once, require_clean_source

MARKER = "TICK_CLOCK_RESULT"
RESULT = re.compile(r"^TICK_CLOCK_RESULT before=(\d+) first=(\d+) final=(\d+)"
                    r" first_irqs=(\d+) irqs=(\d+) monotonic=([01])"
                    r" fallback_stopped=([01]) if_enabled=([01]) if_disabled=([01])$", re.MULTILINE)
PROBE = '''    arch::disable_interrupts();
    if interrupts::hardware_ticks() != 0 { serial::println("TICK_CLOCK_FAILED early-irq"); arch::halt_loop(); }
    for _ in 0..(3 * 4096) { interrupts::poll_fallback_tick(); }
    let before = interrupts::ticks();
    if before != 3 || arch::interrupts_enabled() {
        serial::println("TICK_CLOCK_FAILED fallback"); arch::halt_loop();
    }
    serial::println("TICK_CLOCK_ARMED fallback=3 hardware=0 if=0");
    interrupts::enable();
    let (mut first, mut first_irqs, mut last, mut irqs) = (0, 0, before, 0);
    let (mut monotonic, mut stopped, mut enabled) = (true, true, true);
    for _ in 0..100_000_000 {
        let (tick, polled, raw, masked) = arch::without_interrupts(|| {
            let tick = interrupts::ticks();
            (tick, interrupts::poll_fallback_tick(), interrupts::hardware_ticks(), !arch::interrupts_enabled())
        });
        enabled &= masked && arch::interrupts_enabled();
        monotonic &= tick >= last && polled >= tick;
        last = polled;
        if raw != 0 {
            stopped &= tick == polled;
            if first_irqs == 0 { first = tick; first_irqs = raw; }
        }
        irqs = raw;
        if irqs >= 3 { break; }
        core::hint::spin_loop();
    }
    arch::disable_interrupts();
    irqs = interrupts::hardware_ticks();
    let disabled = !arch::interrupts_enabled();
    let final_tick = interrupts::ticks();
    stopped &= final_tick == interrupts::poll_fallback_tick();
    let disabled = disabled && !arch::interrupts_enabled();
    serial::print("TICK_CLOCK_RESULT");
    for (label, value) in [(" before=", before), (" first=", first), (" final=", final_tick),
        (" first_irqs=", first_irqs), (" irqs=", irqs), (" monotonic=", u64::from(monotonic)),
        (" fallback_stopped=", u64::from(stopped)), (" if_enabled=", u64::from(enabled)),
        (" if_disabled=", u64::from(disabled))] {
        serial::print(label); serial::print_u64(value);
    }
    serial::println(""); arch::halt_loop();
'''


def patch_fixture(root: Path) -> str:
    patches = []
    name = "kernel/src/main.rs"
    path = root / name
    before = path.read_text()
    if MARKER in before:
        raise ValueError("clock fixture already installed")
    after = replace_once(before, "    interrupts::enable();", PROBE + "    interrupts::enable();")
    path.write_text(after)
    patches += difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                   fromfile="a/" + name, tofile="b/" + name)
    # Historical sources predate the read-only diagnostic getter. Add only that
    # observation; never alter their clock selection or IRQ increment logic.
    name = "kernel/src/interrupts.rs"
    path = root / name
    before = path.read_text()
    if "pub fn hardware_ticks()" not in before:
        after = before + "\npub fn hardware_ticks() -> u64 { TICKS.load(Ordering::Relaxed) }\n"
        path.write_text(after)
        patches += difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                       fromfile="a/" + name, tofile="b/" + name)
    return "".join(patches)


def validate_log(log: str) -> None:
    lines = log.replace("\r", "").splitlines()
    results = RESULT.findall("\n".join(lines))
    if len(results) != 1 or sum(MARKER in line for line in lines) != 1:
        raise ValueError("missing, duplicate or malformed clock result")
    before, first, final, first_irqs, irqs, *flags = map(int, results[0])
    if (before != 3 or not 1 <= first_irqs <= irqs or irqs < 3
            or first <= before or final != first + irqs - first_irqs or flags != [1, 1, 1, 1]):
        raise ValueError("clock regressed, stopped or violated IRQ/IF transition")
    proofs = ["BOOT_MODE validation", "BOOT_MEMORY_MAP_VALIDATED", "IRQ_READY",
              "PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx",
              "TICK_CLOCK_ARMED fallback=3 hardware=0 if=0", "IRQ_HARDWARE_ON",
              RESULT.search("\n".join(lines)).group(0)]
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError("missing or duplicate clock phase")
    indexes = [lines.index(proof) for proof in proofs]
    if indexes != sorted(indexes) or lines[indexes[-1]:] != [proofs[-1]]:
        raise ValueError("wrong clock phase or continued execution")
    if any(token in log for token in ("_FAILED", "EXCEPTION_FRAME", "EXCEPTION_TERMINAL",
                                      "KERNEL PANIC", "GENOS_READY", "USERMODE_READY")):
        raise ValueError("clock probe faulted or continued boot")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-ref", help="retained historical source; defaults to clean HEAD")
    parser.add_argument("--timeout", type=int, default=60)
    options = parser.parse_args()
    if options.timeout <= 0:
        parser.error("positive timeout required")
    source = Path(__file__).resolve().parents[1]
    current = require_clean_source(source)
    commit = subprocess.check_output(["git", "rev-parse", "--verify", (options.source_ref or current) + "^{commit}"],
                                     cwd=source, text=True).strip()
    archive = subprocess.check_output(["git", "archive", "--format=tar", commit], cwd=source)
    evidence = source / "build/tick-clock-evidence" / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {"status": "incomplete", "commit": commit, "harness_commit": current,
                "source_clean": True, "archive_sha256": hashlib.sha256(archive).hexdigest(),
                "scope": "real PIT transition; monotonic logical time and caller IF preservation"}
    try:
        manifest.update(environment(firmware_path(), source))
        with tempfile.TemporaryDirectory(prefix="genos-clock-") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tree:
                tree.extractall(root, filter="data")
            fixture = patch_fixture(root)
            (evidence / "fixture.patch").write_text(fixture)
            manifest["fixture_sha256"] = hashlib.sha256(fixture.encode()).hexdigest()
            with (evidence / "build.log").open("w") as output:
                subprocess.run(["cargo", "xtask", "build-test"], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            manifest["image_sha256"] = hashlib.sha256((root / "build/genos.img").read_bytes()).hexdigest()
            boot(root, evidence, options.timeout, halt_marker=MARKER, validate_serial=validate_log)
            manifest["status"] = "passed"
    except BaseException as error:
        manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        manifest["files_sha256"] = {str(path.relative_to(evidence)): hashlib.sha256(path.read_bytes()).hexdigest()
                                    for path in sorted(evidence.rglob("*")) if path.is_file()}
        (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"TICK_CLOCK_PROBE_OK evidence={evidence}")


if __name__ == "__main__":
    main()

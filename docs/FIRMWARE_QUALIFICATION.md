# Firmware lifecycle qualification

This host harness advances roadmap F1.1 through real firmware calls in the
pinned UEFI 0.36.1 helper. It does not install a replacement firmware table or
modify the production retry loop, reset/panic policy, map decoder or loader
allocation handling.

| Task | Cases | Required outcome |
| --- | --- | --- |
| Stale exit-key recovery | `exit-stale-once` | Real INVALID_PARAMETER, a fresh map/key, then successful exit and kernel handoff |
| Stale exit-key exhaustion | `exit-stale-twice` | Two real INVALID_PARAMETER results, then an explicit guest reset without kernel entry |
| In-capacity map growth | `map-grow-small` | An interior page allocation splits the real firmware map; the larger map fits and reaches handoff |
| Excess map growth | `map-grow-large` | Ten alternating interior allocations exceed the map buffer; two real BUFFER_TOO_SMALL results precede guest reset |
| Boot allocation refusal | `alloc-kernel`, `alloc-initrd`, `alloc-boot-info`, `alloc-map-buffer` | Oversized requests receive real firmware denial; the loader shuts down or its existing map-buffer panic path halts |

Run from a clean committed checkout with the pinned Cargo dependencies cached:

```sh
python3 tools/test_firmware_lifecycle.py
python3 tools/test_firmware_lifecycle.py --case exit-stale-once
python3 -m unittest discover -s tools -p 'test_firmware_lifecycle_harness.py'
```

Each case builds a fresh Git archive and private vendored dependencies. Before
patching, every UEFI file must match both its per-file checksum and the original
package archive whose SHA-256 is locked by Cargo. Extra files and symlinks fail
admission. Deliberate argument changes and bounded UART instrumentation are
retained separately with new hashes. The original package receipt is preserved;
the normal repository and Cargo registry sources are not modified.

Map growth allocates alternating interior pages from a checked conventional RAM
span before the first exit attempt. These pages remain LOADER_DATA in the
disposable VM. Later retries perform only map retrieval and exit calls. Allocation
faults change request sizes without fabricating return statuses. The decoder,
whole-image admission, write/copy behavior, two-attempt helper and final failure
handling remain the implementations under test.

Serial evidence requires exact call identities, error/status and phase ordering.
Successful exits must precede kernel entry, map validation and readiness. Failure
cases cannot enter the kernel. QMP must confirm a still-running guest after halt,
or exactly one natural guest shutdown/reset with the expected reason. Host
termination, unexplained reset, paused guest and incomplete/duplicate evidence
never count as success. The runner retains failed cases and attempts the rest.

Per-run source archives, original UEFI packages, modified source/patches, image
hashes, commands, serial/QMP logs and manifests live under
`build/firmware-lifecycle-evidence/`. CI runs the group within its existing
loader-version context and retains every case. Required context names and
repository policy remain unchanged.

This is a bounded firmware fault model, not exhaustive firmware verification.
The real firmware package, allocator and device assumptions remain trusted;
unavailable serial hardware may prevent a complete diagnostic. Tests use the
declared one-CPU VM and never access the user's data volume. Executed results
and reference-profile differences belong in [verification](VERIFICATION.md).

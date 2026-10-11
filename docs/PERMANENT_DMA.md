# Permanent VirtIO allocation and one CPU owner

The production driver groups both split queues and all RX/TX buffers in one
4096-aligned `VirtioDmaMemory`. A `PermanentDma` owns that initialized static
allocation. Discovery must obtain its non-cloneable `DmaClaim` before any PCI,
PIO or MMIO access. The token moves into `VirtioNet`; all queue/buffer pointers
are projected from that token. The four former mutable DMA globals are removed.

`try_claim` requires a static borrow and an irreversible atomic claim. Storage
cannot move or be destroyed while claimed. Failed initialization, unavailable
hardware, fallback, token drop and unwind never release the claim. A second
`NetworkDevice::discover` halts with
`NETWORK_DEVICE_INIT_REJECTED policy=halt` before hardware discovery. Each owner
also denies repeated `VirtioNet::init` before I/O. The outer network coordinator's
separate one-attempt guard remains in force.

The allocation contains 28,672 bytes: two 4096-byte queues, 16,384 RX bytes and
one 4096-byte aligned TX allocation. The wrapper occupies 32,768 bytes because
its claim flag and alignment reserve one additional 4096-byte page. Descriptor
capacity, packet limits and usable buffer bytes are unchanged.

## Unsafe boundary

The primitive exposes raw pointers through a mutable token, never Rust references
to the payload. `PermanentDma<T>` is Sync only for Send payloads: its private
fields and one successful CAS permit exclusive CPU ownership transfer. The token
is Send for Send payloads, is neither Clone nor Sync, and has no release or
reinitialization operation. There is no unsafe Send implementation. All raw
pointer dereferences still require valid allocation bounds, access ordering and
CPU/device ownership; obtaining a pointer does not make those operations safe.

Initialization retains the existing device reset/status protocol before queue
clearing. [TX quarantine](TX_QUEUE_OWNERSHIP.md) and
[RX batch admission](RX_QUEUE_OWNERSHIP.md) retain their failure policies. A
software timeout, token drop or FAILED status does not establish DMA quiescence.
The memory is never freed, scrubbed or returned for reuse after failure. This is
CPU ownership and lifetime enforcement, not a physical-memory capability, mapping
proof, cache-coherence guarantee, hostile-device defense or IOMMU isolation.
PCI/MMIO admission, physical address translation, reset/removal, DMA directions,
interrupt timing and SMP still need broader qualification. F3.2, F4.2/F4.4,
F6.4 and H-DMA remain open.

## Evidence

Six pure tests exercise exclusive claim, irreversible drop/unwind, competing
threads with a Send-but-not-Sync payload, independent aligned allocations,
retained tail-byte provenance, denied-claim byte preservation and token movement.
They join the pinned Miri scope. Four compile-fail examples deny movable storage,
token cloning/sharing and a non-Send shared allocation. The production driver's
native fixture uses separate permanently live allocations and retains the actual
TX/RX regression assertions; it does not execute device DMA.

The software `network-device-repeat` VM fixture calls the driver directly after
the first unavailable network discovery, bypassing the outer initialization
guard. It requires the distinct driver refusal and rejects continuation, faults,
reset or stopped-QEMU substitutes. No NIC is attached. Archived source
`d203b4322a1d7a5440eff89aa6e6c5c6972ca4f4` reproduces the former second-discovery
continuation under `build/permanent-dma-old-counterexample/`; the current strict
validator rejects that execution. Qualification identities and results belong in
[the verification ledger](VERIFICATION.md).

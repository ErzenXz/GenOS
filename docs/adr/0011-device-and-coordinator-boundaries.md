# ADR-0011: Isolate device ownership from protocol and terminal work

- **Status:** Proposed; records the current QEMU driver boundary.
- **Date:** 2026-09-23
- **Decision owners:** network/storage/driver maintainers
- **Related roadmap:** F4.5, F5.2, H-VM, H-DMA, H-IOMMU
- **Supersedes:** None

## Decision

Use the named `genos-q35-tcg-v3` virtual devices as the supported local
reference: serial COM1 for the active console, a separate PCI IDE/ATA PIO
data volume, and a modern VirtIO 1.x PCI NIC with legacy mode disabled.
Drivers own registers, DMA/PIO buffers and queue descriptors; the network
stack receives bounded owned Ethernet frames, not a descriptor pointer.
NIC MSI-X IRQ records readiness and acknowledges the interrupt, while the
coordinator validates used-ring IDs/lengths, copies frames, returns buffers
and advances protocols. The ATA path uses bounded 512-byte LBA28 PIO and
explicit `FLUSH CACHE`; successful persistent writes require the storage
commit protocol in [ADR-0012](0012-bounded-gfs2-snapshot-commit.md).

Modern VirtIO is chosen over making NE2000 the normal device because its
versioned capabilities and queues provide the target transport. A separately
labelled NE2000 PIO fallback remains for compatibility/recovery research; it
cannot satisfy a modern-driver proof. Missing NIC is nonfatal and leaves the
local shell usable. Missing/invalid storage leaves temporary files and
read-only status usable; it never silently formats unreadable media.

## Failure and authority boundary

Descriptor IDs, lengths, feature bits and ownership transitions are checked
before frame publication. Failed TX/RX or storage commands return bounded
failure; a failed/uncertain persistent flush quarantines the volume rather
than treating a retry as a new acknowledged commit. Drivers may not keep a
borrow into a user buffer after syscall admission, and IRQ handlers may not
perform potentially blocking device work. User handles authorize operations
through process ownership; PCI addresses and DMA descriptors are not user
capabilities.

The current VirtIO DMA memory is identity-mapped and assumes a trusted
emulated device. There is no IOMMU policy or containment against a malicious
device, and no physical-machine qualification. Those are explicit later gates,
not implications of a working QEMU queue. ATA PIO avoids DMA on the selected
storage path but does not prove physical cache honesty or power-loss behavior.

## Compatibility, alternatives and rollback

Changing a negotiated VirtIO feature, queue layout, storage controller or
cache policy requires a new reference profile and repeated transport, IRQ,
fault and persistence evidence. Offloads/multiqueue/NVMe are deferred until
their owner/buffer/reset contracts are reviewed. Rollback uses a matched old
kernel and its declared VM/device profile; reset and reconstruct device
queues, never reuse live descriptors or import controller state. Retain the
original data image read-only during a storage-provider migration as required
by [ADR-0008](0008-bounded-littlefs-growth.md).

The [network device contract](../NETWORKING.md), [storage device contract](../STORAGE.md),
[reference profile](../REFERENCE_VM.md), and production
`kernel/src/network_device.rs` / `kernel/src/storage.rs` define the exact
current implementation. Tests cover modern/fallback marker separation,
descriptor bounds, selected packet loss and storage error/recovery schedules;
physical reset, untrusted DMA and worst IRQ latency remain open.

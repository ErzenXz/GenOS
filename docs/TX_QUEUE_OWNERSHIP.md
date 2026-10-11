# Single-buffer VirtIO TX ownership

The current split TX queue publishes descriptor zero with one permanent 2048-byte
buffer. Queue capacity remains eight, but only one TX descriptor may be in flight.
`kernel/src/tx_queue.rs` owns the available/used counters and publication phase;
`network_device.rs` invokes it before touching the TX buffer or descriptor.

Previously, `transmit` returned false on its polling deadline without retiring
or quarantining that publication. A later call overwrote the same bytes and
descriptor even though the device might still be reading them. It also accepted
any changed used index, rather than exactly one completion. These are ownership
defects independent of the network protocol's retry policy.

## Transition and failure contract

- Idle to device-owned: reserve descriptor zero before the first CPU buffer write,
  write the packet/header and descriptor, publish the next available entry with
  the existing release ordering, and notify the device once.
- Device-owned to idle: after the existing completion observation ordering,
  require exactly one wrapping used-index advance and descriptor ID zero.
  Only that completion permits a subsequent call to reuse the buffer.
- Timeout, invalid advance or wrong ID: quarantine permanently for this instance.
  Further TX/RX entry returns false before buffer/descriptor/ring mutation or
  notification. A late apparently valid completion cannot undo quarantine.
  Rejected completions do not advance the owned used counter or increment
  successful-TX/completion metrics.

A rejected second reservation leaves the in-flight state unchanged. Input length
admission precedes reservation. Ring indices wrap at 16 bits and ring slots use
the same eight-entry constant as the production descriptor arrays.

This follows the split-ring ownership and reset boundaries in
[VirtIO 1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html),
sections 2.4 and 2.7: a timeout alone does not return a live descriptor to the
driver. The implementation adds no reset/restart path. It does not claim that
quarantine stops DMA or that setting FAILED would establish quiescence. Existing
queue/buffer storage remains permanently reserved; it is not freed, scrubbed or
reused by the CPU after quarantine. Global network initialization remains limited
to one attempt by the [coordinator adapter](COORDINATOR_ACCESS.md).

## Evidence and limits

Six pure production-state tests cover successful retirement, busy reservation,
timeout plus late completion, wrong IDs, stale/skipped/excess used advances,
unsolicited/duplicate completion and 16-bit wrap. They are included in the pinned
Miri scope. The production driver host fixture also executes real `transmit`
polling for valid completion, wrong ID, excess advance and timeout. It then
injects a late valid completion and checks that repeated send/receive preserve
TX buffer bytes, descriptor fields and the available index. Notifications target
borrowed RAM; no host device or hardware DMA executes in this fixture. Its
existing coalesced RX and wrap/invalid-advance assertions remain intact.

The old production path's timeout overwrite is reproduced on source
`719757d7763d373367e45719682950dbb0e1d358` with an archived-source host fixture;
the failing assertion, patch, source/archive identity and log hash remain under
`build/tx-timeout-counterexample/`. Qualification runs for the corrected source
are recorded in [the verification ledger](VERIFICATION.md).

The pure state machine supplies no physical-buffer capability or hardware proof.
The coordinator still provides CPU-side exclusivity; the actual driver still
trusts coherent x86 device behavior and admitted mappings. The [permanent allocation claim](PERMANENT_DMA.md) now removes raw DMA globals
and rejects repeated discovery before device I/O. Broader device ownership,
PCI/MMIO admission, reset
completion, device removal, IOMMU isolation and SMP require further work. This
fix does not close F3.2, F4.2, F4.4, F6.4 or H-DMA, or change the frozen VM profile.

The subsequent [RX batch boundary](RX_QUEUE_OWNERSHIP.md) validates a complete
captured prefix before any packet copy or descriptor return. It does not add
DMA isolation or reset/reuse authority.

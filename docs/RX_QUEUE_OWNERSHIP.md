# VirtIO RX batch ownership

The split RX queue has eight permanent 2048-byte device-writable buffers. A used
ring snapshot can contain up to eight completed descriptors. The coordinator
must finish validating that snapshot before copying a packet or returning any
of its descriptors to the device.

The previous path validated one completion at a time and immediately reposted
its buffer. A repeated ID in the same snapshot could then make the coordinator
copy that buffer again while the device could already be writing to it. A
bounded used-index advance alone did not prevent this ownership error.

## Admission and draining

`RxBatch<N>` in `kernel/src/rx_queue.rs` owns a copied array of completion records,
the consumed used counter and the cursor within the admitted batch. The driver
uses N=8, matching its descriptor arrays. After reading the device's used index,
the driver applies acquire ordering and captures that entire bounded prefix.
Admission checks the exact wrapping advance/count, each ID against capacity,
ID uniqueness throughout the prefix, and each raw DMA length against the
12-byte header minimum and 2048-byte backing extent. A failure quarantines the
instance before its first packet copy, descriptor repost or success counter.

Once admitted, the snapshot is immutable until drained. Each take advances one
consumed counter; the driver copies that owned buffer into the stack's packet
before reposting its descriptor with release ordering and notification. A new
IRQ while the batch is pending remains ready for the next snapshot. Replacing
the pending snapshot is refused. A descriptor may legitimately occur again in
a later snapshot after its earlier copy and repost; that is not a duplicate in
one captured ownership interval.

Packet size admission remains separate. A raw DMA extent that fits its buffer
but exceeds the stack's unchanged 1518-byte frame budget is dropped without
copying into the caller's packet, then reposted as before. It does not quarantine
the queue. A malformed completion ID, duplicate or backing extent does quarantine
RX; later RX/TX entry returns before queue/buffer mutation or notification, even
if a later used record appears valid. The caller's existing packet bytes, length
and owner remain unchanged on those rejected paths.

The completion and exposed-buffer rules come from the split-ring and cleanup
sections of [VirtIO 1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html),
sections 2.7 and 3.3.1. Quarantine does not establish device reset completion or
DMA quiescence. All queue/buffer storage remains permanently reserved; the
implementation does not free, scrub or reconstruct it after a queue fault.

## Evidence and caller obligations

Seven pure production tests cover ordered drain, all 28 duplicate-position pairs
in a full batch, malformed ID/extent after a valid prefix, mismatched/stale/excess
advances, pending-batch replacement denial, legitimate descriptor reuse in the
next snapshot, empty/wrapping batches, irreversible quarantine and the separate
oversized-packet policy. They are in the pinned Miri scope.

The single production driver host fixture retains all TX timeout/completion and
RX coalescing/wrap checks. It now adds new IRQ/completion arrival during a pending
batch, duplicate/invalid ID and extent rejection before a valid prefix is copied,
late-record denial, unchanged packet and available-ring contents/indices, no
success/notification increments on rejection, and oversized drop/repost without
quarantine. No concurrent host test or device owns those static fixture buffers;
notifications target borrowed RAM. It does not execute device DMA or MMIO/PIO.

An archived-source counterexample on
`6d149ad73b6602d3f935744dd40e641687777ee7` demonstrates the old production path
accepting a duplicate-ID batch and advancing available.index from eight to nine.
The failing assertion, patch, exact source/archive identity and log hashes remain
under `build/rx-duplicate-counterexample/`. Corrected qualification artifacts are
recorded in [the verification ledger](VERIFICATION.md).

This pure batch type is bookkeeping, not a physical-buffer capability. The
coordinator must remain exclusive; the driver must capture records in order,
copy a taken buffer before reposting it, preserve the device-read/write access
directions and never fabricate completion or reset authority. The current driver
still assumes an admitted coherent x86 platform and a trusted device that obeys
VirtIO ownership. A malicious device can ignore descriptors or write after
completion; this change supplies no IOMMU isolation. Raw DMA globals, device
claim/initialization, PCI/MMIO admission, reset/removal, SMP and broader interrupt
timing still need review. F3.2, F4.2/F4.4, F6.4 and H-DMA remain open.

# Network and hardware choices for the console-first roadmap

Researched 2026-09-08 against the primary pages below. Recommendations are GenOS
planning decisions, not implemented features, compliance claims or performance
results. Local implementation context is `5599dc7`; see [networking](../NETWORKING.md)
and [IPv6](../IPV6.md) for the bounded behavior already delivered.

## Primary-source findings

| Source inspected | Relevant finding | Consequence for GenOS |
| --- | --- | --- |
| [RFC 9293, TCP](https://www.rfc-editor.org/rfc/rfc9293.html) | This consolidated TCP specification includes a requirements summary and the connection/stream state machine; it also points to companion congestion and timing specifications. | Maintain a requirement-to-test matrix for the declared network profile rather than treating a successful HTTP exchange as complete TCP. |
| [RFC 6298, retransmission timer](https://www.rfc-editor.org/rfc/rfc6298.html) | RTO estimation includes smoothed RTT and variation, rules for ambiguous retransmitted samples, timer management and exponential backoff. | Test timing in a controllable clock model before tuning for one low-latency VM network. |
| [RFC 5681, congestion control](https://www.rfc-editor.org/rfc/rfc5681.html) | Congestion behavior covers slow start, congestion avoidance and loss recovery; a receive window is not a congestion window. | Use a documented conservative baseline before considering alternative high-throughput algorithms. |
| [RFC 8504, IPv6 node requirements](https://www.rfc-editor.org/rfc/rfc8504.html) | IPv6 host behavior extends beyond address acquisition to neighbor/ICMP behavior, transport use and path-MTU considerations. It distinguishes minimal implementations from broader host requirements. | Keep SLAAC/DAD/router echo marked delivered, but leave IPv6 sockets, DNS, reachability and PMTU requirements open. |
| [RFC 9413, maintaining robust protocols](https://www.rfc-editor.org/rfc/rfc9413.html) | Robustness requires maintaining specifications and implementations, with care around liberal acceptance of problematic input. | Validate malformed input; separately document which valid optional features the bounded profile does not support. Do not call every unsupported packet malformed. |
| [Google packetdrill](https://github.com/google/packetdrill) | Packetdrill scripts connect application operations, wire packets and timing; its remote mode includes more of the system and more timing variability. | Borrow scripted wire traces and timing-aware assertions. Its existing syscall/TUN integration is not a drop-in GenOS test adapter. |
| [QEMU VirtIO guidance](https://www.qemu.org/docs/master/system/devices/virtio/index.html) | QEMU recommends VirtIO devices for guests that are not specifically testing historical device drivers. | Prefer modern VirtIO block/network/console in the future VM profile; retain explicit legacy recovery where needed. This does not require a GPU before a console milestone. |
| [VirtIO 1.2 CS01](https://docs.oasis-open.org/virtio/virtio/v1.2/cs01/virtio-v1.2-cs01.html) | This inspected document is a Committee Specification. It defines feature negotiation, queues and reset completion; drivers must observe reset completion before reinitializing. | Pin this published revision as the initial reviewed baseline and map each used feature to its requirements and negative tests. Audit current behavior instead of claiming conformance from a version label. |
| [VirtIO 1.3 CSD01](https://docs.oasis-open.org/virtio/virtio/v1.3/csd01/virtio-v1.3-csd01.html) | The existing repository's 1.3 “latest stage” URL currently identifies Committee Specification Draft 01, dated 2023-10-06. | Label its draft status accurately. Any draft-specific feature needs an explicit compatibility decision; “newest” is not automatically the best baseline. |
| [UEFI specifications index](https://uefi.org/specifications) | The index lists UEFI 2.11 and ACPI 6.6, with dated releases. | Record exact revisions used by each implementation/review; do not silently follow an unversioned specification link. The ACPI index establishes availability, not GenOS hardware support. |
| [UEFI 2.11 boot services, §§7.2.3/7.4.6](https://uefi.org/specs/UEFI/2.11/07_Services_Boot_Services.html) | The firmware returns descriptor size/version and a changing map key; an invalid exit key requires the map/exit sequence to be retried under the documented restrictions. | Audit both the UEFI crate handoff and GenOS's copying/validation of BootInfo. No silent descriptor truncation or assumptions that all firmware memory is usable. |
| [RFC 8446, TLS 1.3](https://www.rfc-editor.org/rfc/rfc8446.html) | TLS specifies authenticated handshake and record protection, with implementation guidance for randomness and certificate authentication. | Integrate maintained reviewed cryptography in userspace only after entropy, clock, trust and stream interfaces exist; do not invent a new protocol or present plaintext diagnostic HTTP as secure networking. |

The UEFI 2.11 handoff text corroborates the older 2.10-A text inspected in the
[kernel research](2026-09-kernel-foundations.md). The ACPI 6.6 HTML landing page
could not be retrieved by the browser tool; detailed ACPI implementation claims
are therefore not derived from that page. The official index was read. QEMU's
master docs are research material, not a replacement for pinning the tested QEMU
binary and machine version in an evidence manifest.

## GenOS decisions recommended now

1. **Declare network profiles.** The first dependable local-console VM can be
   explicitly offline. A network-enabled experimental profile must keep its
   finite concurrency and unsupported features visible. General-network claims
   require the TCP/IPv6 and security gates, not just adding a `ping` command.
2. **Complete existing state machines before expanding throughput.** Preserve
   exact owner/incarnation/handle/request/tuple identity. Exercise retransmission,
   duplicate data, out-of-order segments, wraparound, half-close, simultaneous
   close, RST, zero-window/persist, slow readers, timeout and cancellation. Bound
   every byte buffer, queue, retry and per-owner share. Test two different owners,
   not only two clients of one listener.
3. **Test the entire driver lifecycle.** Separate CPU memory ordering from
   volatile MMIO and DMA visibility. On reset/cancel, do not reuse device-owned
   buffers until quiescence is established. If reset completion is unknown, retain
   ownership and report device failure rather than freeing storage optimistically.
   Cover invalid used indices/lengths, lost/coalesced interrupts, failed negotiation,
   timeout, repeated reset and offline boot without a silent fallback success.
4. **Use a versioned VM reference before hardware breadth.** Pin Rust, QEMU,
   machine/CPU features, firmware hash, image hash, memory, devices and test-network
   configuration. Keep a separate feature/fault matrix. Add virtio-blk after its
   DMA/flush contract; do not couple console acceptance to virtio-gpu, Wi-Fi,
   audio, laptop power management or full SMP.
5. **Qualify one physical machine deliberately.** First capture a read-only
   discovery report and select the actual firmware, storage, interrupt and input
   path. Add NVMe/xHCI/APIC support against published device specifications and
   chosen hardware; test timeout/reset/flush and recovery on that machine. Virtual
   MSI-X success is not physical-device or IOMMU isolation proof.

## Proposed acceptance evidence

- A normative requirement matrix with implemented, unsupported and untested rows,
  source revision, parser/state owner, positive case and negative case.
- Scripted packet cases with packet capture, deterministic timing where possible,
  explicit wall-clock tolerances where not, sequence/queue counters and exact
  application-visible outcomes. Never relax correctness to satisfy a fast test.
- Concurrent owners cannot receive each other's bytes or readiness; a stalled peer
  cannot consume another owner's budget. Closing/cancelling a request makes later
  completion inert and restores all documented resource counts.
- IPv6-only and dual-stack DNS/UDP/TCP tests, route/neighbor expiry, malformed
  advertisements, extension policy and PMTU/error behavior before dual-stack claims.
- A reproducible throughput/latency/resource baseline before queue batching,
  offloads, multiqueue or alternate congestion algorithms become defaults.
- A physical-hardware report naming firmware, CPU, devices, exact GenOS image,
  interrupts, reset/error outcomes, data recovery and unsupported functions.

These choices preserve GenOS's own architecture. Reusing a specification or test
method does not imply Linux ABI compatibility, a Linux implementation dependency,
or a universal performance advantage.

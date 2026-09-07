# Networking

GenOS 0.55 combines modern VirtIO transport, asynchronous ABI 16 UDP/TCP clients, and ABI 18 TCP listener/readiness authority with bounded concurrent passive service: four global handshake and accepted-stream slots, at most two per process incarnation, keyed by exact peer tuple. QEMU presents a VirtIO 1.x PCI network device with its legacy interface disabled, and the kernel runs DHCP, ICMP, DNS, Ring 3 UDP/TCP client requests, deterministic sustained-loss/reordering recovery, a 1,024-byte accepted stream, and two host-forwarded inbound clients through that device. No host socket shortcut, firmware network stack, guest agent, or Linux component handles guest packets.

This is a modern device foundation with bounded client and bounded concurrent server transactions. It is not yet a production Internet stack; multi-segment outbound flight, arbitrary receive windows, more than two clients on one listener, readiness sets, broader fault matrices, physical-device interrupt proof, IPv6, and TLS remain explicit gates below.

## Device boundary and selection policy

The protocol stack depends on a frame-device interface providing initialization, MAC discovery, bounded transmit, and bounded receive operations. It does not contain NE2000 registers, VirtIO PCI capabilities, virtqueue descriptors, or device-specific DMA rules.

Device selection is ordered and observable:

1. discover a VirtIO network PCI function;
2. require its modern vendor capabilities and `VIRTIO_F_VERSION_1`;
3. configure it and publish `NETWORK_DEVICE_READY driver=virtio-net-pci transport=modern-pci`;
4. only if no usable modern device exists, attempt the isolated NE2000 recovery driver and label it `ne2000-pio-legacy-fallback`;
5. continue booting without networking if neither device exists.

Normal `cargo xtask run` and every network smoke phase use `virtio-net-pci,disable-legacy=on`. The tests require the exact modern-driver marker, so a legacy fallback cannot accidentally satisfy the release gate. NE2000 is retained for compatibility and recovery research only; it is not the default, the performance target, or evidence that a modern-network milestone passed.

## VirtIO 1.x PCI transport

The kernel scans PCI configuration space for a VirtIO network function, enables memory-space and bus-master access, and follows the vendor capability chain to locate the common configuration, notification region, and device configuration. Initialization follows the VirtIO status sequence:

- reset and observe status zero;
- set `ACKNOWLEDGE` and `DRIVER`;
- read device features;
- require and negotiate `VIRTIO_F_VERSION_1` and `VIRTIO_NET_F_MAC`;
- deliberately decline checksum, segmentation, mergeable-buffer, and other offloads until their contracts exist in the stack;
- set and verify `FEATURES_OK`;
- configure queues and then set `DRIVER_OK`.

RX queue 0 and TX queue 1 are independent eight-entry split virtqueues. Descriptor tables, available rings, used rings, and frame buffers are explicitly aligned and identity-mapped for DMA on the current x86_64 boot contract. Queue publication and consumption use volatile accesses plus release/acquire fences. Notifications use each queue's device-provided notification offset and multiplier.

Every receive descriptor owns a 2048-byte device-writable buffer. Modern VirtIO's 12-byte `virtio_net_hdr`, including `num_buffers`, precedes the Ethernet frame. Incoming lengths and descriptor IDs are validated before copying into the stack-owned 1518-byte frame buffer, after which the receive descriptor is returned to the device. TX prepends a zeroed 12-byte header, pads short Ethernet frames, waits for completion with a fixed deadline, and never reuses its DMA buffer while the device owns it.

GenOS programs PCI MSI-X table entry 0 for IDT vector 48 and assigns RX queue 0 and TX queue 1 to that shared vector. The interrupt stub records one readiness epoch and acknowledges the local APIC; it never validates descriptors, copies packets, or changes ownership. The network coordinator performs those operations after observing interrupt readiness. TX consults the used ring after the interrupt epoch advances. RX normally consults it after interrupt readiness, with one recovery inspection per 32,768 receive calls for lost-interrupt recovery. Recovery polls and completions are counted and release-gated. Per-queue vectors, multiple queue pairs, batching, offloads, and physical-device routing remain open.

The transport follows the [OASIS VirtIO 1.3 specification](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html). QEMU also recommends VirtIO device models when a guest is not specifically testing historical hardware; see the [QEMU VirtIO documentation](https://www.qemu.org/docs/master/system/devices/virtio/index.html).

## Packet ownership

Frames move through explicit free, driver, and stack ownership states. The device owns published DMA descriptors. The driver validates completed descriptor metadata and copies one bounded frame into stack-owned storage. Protocol values that outlive a receive iteration copy their bytes into owned state, and the descriptor is recycled only after the copy completes.

The fallback NE2000 driver preserves the same ownership contract with one receive buffer, while VirtIO keeps eight receive buffers available so a single packet does not immediately starve the device.

## Protocol baseline

The current stack implements:

- Ethernet II framing;
- ARP requests and replies for a destination or configured gateway;
- IPv4 without fragmentation or reassembly;
- ICMP echo request/reply;
- UDP datagrams;
- DHCP discover, offer, request, and acknowledgment;
- DNS A queries over UDP;
- one active-open TCP exchange with SYN, SYN-ACK, ACK, data, FIN, and RST handling;
- up to two concurrent passive SYN/SYN-ACK/ACK handshakes feeding exact bound listeners, and up to two concurrent accepted streams that remain open across multiple bounded request/response cycles before peer half-close and guest FIN.

IPv4 headers and ICMP, UDP, and TCP payloads are checksum validated. IPv4 fragments, invalid header lengths, inconsistent total lengths, malformed UDP lengths, invalid TCP data offsets, and truncated DNS names are rejected. Host tests feed every truncation of a valid frame plus malformed length and header combinations through the parsers.

QEMU's reference user network assigns `10.0.2.15`, gateway `10.0.2.2`, and DNS `10.0.2.3`. On-link destinations are resolved directly with ARP; off-link destinations resolve the gateway. Storage-only smoke phases intentionally omit a NIC, and `NETWORK_DEVICE_UNAVAILABLE` remains non-fatal.

## ABI 15 exchange API

ABI 15 exposes three bounded calls:

- `network_config` copies the current IPv4 configuration and MAC address into validated process memory;
- `udp_exchange` sends one datagram and returns the first matching response from the exact address and port;
- `tcp_exchange` opens one connection, sends one request, collects a bounded response, acknowledges received data, and closes after the peer's FIN.

Every request is copied from a validated caller-owned mapping before the device is touched. Every response is copied back only within the validated output capacity.

`SHELL.ELF` uses the UDP API to construct and send a DNS A query for `example.com`, parses the answer in Ring 3, then uses the TCP API to send an HTTP/1.1 request with a required `Host` header to the deterministic xtask server through QEMU's `10.0.2.2` host alias. It requires `HTTP/1.1 200` and `GENOS_OK` before emitting `USER_HTTP_REQUEST_OK` and `USER_SOCKET_API_READY`. This plaintext endpoint exists only inside the deterministic test network and is not an approved Internet security boundary.

## ABI 16 asynchronous clients and ABI 18 listeners/readiness

ABI 16 adds process-owned UDP and TCP-stream objects. `socket_open` returns an opaque generation-safe handle registered as a socket in the caller's unified typed capability table. `socket_connect`, `socket_send`, `socket_receive`, `socket_status`, `socket_shutdown`, and `socket_close` require that exact live capability.

Each process may own four socket handles. Every socket has separate fixed 128-byte send and receive queues. Sends are admitted only while capacity exists; saturation returns `USER_ERROR_WOULD_BLOCK` and never overwrites queued bytes. Receives preserve unread suffix bytes after a partial copy and return `WOULD_BLOCK` when no data is available. Status exposes protocol, lifecycle state, readiness bits, and exact queued-byte counts. Shutdown clears only the selected direction's queued work, close revokes the handle, and process termination reclaims every socket owned by that exact process incarnation.

For UDP, `ProcessManager` moves one admitted datagram into an in-flight slot and assigns a nonzero monotonic request ID. The request records the exact process slot, incarnation, task, PID, socket handle, destination, port, and copied payload. `RuntimeCoordinator` revalidates that full identity before starting or completing transport. One bounded coordinator slot resolves the next hop, sends the datagram, consumes at most one received frame per coordinator tick, validates the response address, port, IPv4 header, and UDP checksum, then places at most 128 response bytes into the owning socket's receive queue. The syscall itself only admits or reads bounded queue data; it does not wait for the network.

The UDP state machine allows three attempts separated by scheduler deadlines. Timeout clears the in-flight request, marks the socket `Failed`, and exposes error readiness. Write shutdown or close invalidates the exact request; the coordinator cancels its packet operation and drops any stale completion. A second transport request cannot overwrite the occupied coordinator slot. These limits are deliberate: per-process socket count, send/receive bytes, in-flight bytes, request copies, response copies, retry count, NIC polls per tick, and concurrent coordinator transports are all fixed.

`SHELL.ELF` sends a real DNS A query through `socket_send`, yields while the asynchronous client coordinator progresses it, receives the answer through `socket_receive`, and validates it in Ring 3 before emitting `USER_SOCKET_UDP_ASYNC_READY`. It separately proves bounded timeout and write-shutdown cancellation. The smoke suite requires the start, completion, timeout, cancellation, and current ABI 18 capability markers on both modern VirtIO network boots.

For TCP, the same exact request identity owns one bounded client transaction. The coordinator resolves ARP, validates the exact SYN-ACK acknowledgment, sends ACK plus at most 128 request bytes, and accepts response segments only from the exact IPv4 address and port with valid checksums, acknowledgment, and next sequence. In-order bytes accumulate into a fixed 128-byte response; duplicate or out-of-order segments receive the current cumulative ACK without entering the queue. FIN is acknowledged and followed by an active close. RST, overflow, exhausted retry deadlines, and invalid completion authority mark the socket `Failed`.

`SHELL.ELF` sends an HTTP/1.1 request with `socket_send`, yields while the coordinator progresses TCP, then validates the 65-byte response returned by `socket_receive` before emitting `USER_SOCKET_TCP_ASYNC_READY`. The deterministic host accepts a second connection for the retained ABI 15 compatibility exchange. Without that server, QEMU returns a real RST; the socket exposes error readiness and the operating system continues booting. A separate in-flight write shutdown proves protocol-specific cancellation and stale-completion rejection.

The TCP client path is deliberately one bounded request/response transaction, not a general long-lived byte stream.

ABI 18 retains TCP-only `socket_bind`, `socket_listen`, and non-blocking `socket_accept` and adds `socket_wait`. Ports below 1024 are reserved; every admitted local port has one owner across all live process socket sets. A listener owns a fixed backlog of at most two pending peers. Four passive slots exist globally, with a hard two-slot ceiling per process incarnation. Empty accept returns `USER_ERROR_WOULD_BLOCK`; a queued peer makes the listener readable and accept-ready; an accepted peer becomes a fresh generation-safe TCP capability registered in the caller's unified typed handle table. Child allocation failure preserves the pending peer, typed-table registration failure rolls the child back, and close or process cleanup releases the port.

`SHELL.ELF` first proves the ABI 18 authority contract: low-port and oversized-backlog calls are rejected, an altered handle grants nothing, duplicate bind returns unavailable, empty accept returns `WOULD_BLOCK`, a closed listener is stale, and the same port can be rebound after close. It then binds port 18081, announces `USER_SOCKET_PASSIVE_LISTEN_READY`, and runs a bounded service window. Accept, receive, drain, and close phases wait on exact readiness masks with two-tick safety deadlines instead of sleeping and checking status again. The coordinator accepts only exact checksum-valid SYNs for live destination ports, validates each peer tuple, and queues each established peer through the exact process slot, incarnation, PID, handle, and port. Missing listeners, saturated backlogs, owner-budget exhaustion, and connections beyond the fixed global slot budget receive refusal resets.

Passive service is concurrent within fixed budgets. Four global handshake slots and four global stream slots are keyed by exact peer tuple, while the two-slot per-process ceiling prevents one owner from consuming the full transport budget. One shared bounded receive pump decodes each frame exactly once and routes it to the owning stream slot, handshake slot, or a four-cell pending-SYN queue, so concurrent operations never consume each other's frames. A handshake slot admits and acknowledges at most one 128-byte early data segment plus FIN arriving between the peer's final ACK and stream attachment, then seeds the accepted stream with those exact bytes. Stream slots are served in rotation; a slot with unread Ring 3 bytes re-acknowledges its current sequence without admitting new data, so a slow reader neither loses bytes nor starves the sibling stream.

Each completed handshake's peer MAC, IPv4 address, ports, and initial sequence numbers travel unchanged through backlog admission into its accepted child. A stream slot accepts payload or FIN only with the exact tuple, checksum, bounded sequence window, and valid acknowledgment. One 128-byte delivery buffer and one 128-byte deferred segment provide a fixed 256-byte receive budget. A gap never advances the cumulative acknowledgment; when the missing segment arrives, the deferred bytes become acknowledged but are promoted to Ring 3 only after the earlier buffer is consumed. Duplicates receive the current cumulative acknowledgment without duplicate delivery. Every ACK advertises the actual remaining fixed receive storage, including zero when both buffers are occupied.

A Ring 3 send moves at most 128 bytes into a separate in-flight record with a nonzero request ID bound to the exact process slot, incarnation, task, PID, handle, peer, and byte count; completion waits for the exact cumulative ACK. ACK latency updates a bounded per-stream retransmission timeout, and retries use bounded exponential backoff under the existing hard retry ceiling. An accepted capability may perform another receive/send cycle after completion. Oversized input resets the peer, stale authority cancels with RST, and response/FIN retransmission plus per-slot idle lifetime are bounded. A stream that resets or times out is dropped without disturbing its sibling, and the Ring 3 window closes the failed capability so a replacement client can be accepted.

The deterministic QEMU phase connects two real host `TcpStream` clients through QEMU forwarding once the listener marker appears. One client performs three consecutive exchanges whose first response transmissions are deliberately dropped; every timeout halves the per-stream slow-start threshold, resets the congestion window to one 128-byte MSS, and recovers within the fixed retry ceiling. The other begins with a 256-byte reordered request and then completes six more 128-byte exchanges, for 1,024 received and 1,024 returned bytes through the same accepted capability while resident receive storage never exceeds 256 bytes. Both clients half-close and require EOF after guest FIN. The acceptance marker includes congestion events, min/max congestion window, total stream bytes, and the earlier interrupt and buffer budgets. A separate boot without host forwarding must still reach `GENOS_READY` and is built without fault injection.

`socket_wait` validates the exact owned handle, a non-empty readiness subset, and a deadline of at most 10,000 ticks. If readiness already intersects the mask, the call returns those bits. Otherwise the process enters `Waiting`; before each scheduling pass the process manager scans from a rotating cursor and wakes at most two ready, timed-out, or stale waiters. The live passive service requires block and wake markers, and timeouts return the distinct `USER_ERROR_TIMED_OUT`. The ABI currently waits on one handle, not a set.

## Loss and timeout policy

DHCP, ARP, UDP response waits, active TCP SYN, passive TCP SYN-ACK, active request data, passive response data, and passive FIN allow bounded retries. Compatibility exchanges use a fixed poll budget and return `USER_ERROR_UNAVAILABLE` after exhaustion. Passive response ACK latency updates a bounded retransmission timeout; retry deadlines back off without exceeding the hard budget. Asynchronous paths expose `Failed` plus error readiness after exhaustion instead of leaving a socket in flight. Every passive stream slot has its own fixed idle lifetime so an accepted but inactive or malformed peer cannot monopolize a transport slot indefinitely or starve its sibling. RST fails immediately. QEMU requires three consecutive deliberate first-response losses, one reordered two-segment delivery, refused TCP compatibility, asynchronous TCP RST, ABI 16 UDP timeout, protocol-specific cancellation, and the complete concurrent passive request/response/close marker sequence.

## Required modernization milestones

The following are release gates, not optional ideas:

1. **Larger accepted TCP streams and broader concurrency:** add multi-segment outbound flight, readiness sets, a live multi-process server gate, and more than two concurrent clients on one listener.
2. **Production TCP behavior:** add larger dynamic windows, broader RTT/loss behavior, more out-of-order slots, fast retransmit, selective acknowledgments where negotiated, and duplication/delay/zero-window/exhaustion tests.
3. **VirtIO performance expansion:** prove MSI-X on reference hardware, add per-queue vectors, multiple queue pairs where useful, measured batching, and carefully negotiated checksum/segmentation offloads with fallback tests.
4. **IPv6 dual stack:** IPv6 parsing and routing, ICMPv6, neighbor discovery, router advertisements, SLAAC, DNS AAAA, path-MTU handling, and dual-stack policy tests. IPv4 remains supported but cannot be the only production path.
5. **Secure networking:** kernel entropy first; TLS 1.3 and certificate validation in isolated userspace; HTTPS; trust-store/update policy; time validation; and negative tests for expired, mismatched, revoked, malformed, and untrusted certificates. GenOS will not invent its own cryptography.

No application that handles credentials, tokens, personal data, updates, or packages may treat the current plaintext exchange API as an approved transport.

## Verification

`cargo xtask test-network` builds GenOS, starts a deterministic host HTTP/1.1 server, and boots QEMU with modern-only VirtIO PCI networking. It requires:

- VirtIO 1.x feature, MSI-X vector 48, and split-queue readiness;
- the exact modern driver and transport marker;
- eight-buffer packet ownership readiness;
- DHCP configuration and ICMP echo;
- Ring 3 DNS resolution;
- the ABI 18 Ring 3 socket-capability lifecycle, scheduler readiness waits, listener authority, duplicate-bind refusal, empty accept, close/rebind cleanup, and forged/stale denial;
- the explicit listener-ready marker followed by real host-forwarded passive SYN/SYN-ACK/ACK exchanges, exact backlog admission, and Ring 3 accepted-child capabilities;
- one host client recovering a deliberately dropped response and one host client receiving two reversed logical segments in order through the same Ring 3 capability, followed by peer half-close, guest FIN acknowledgment, and EOF;
- `NETWORK_REGRESSION_BUDGET_OK` with bounded bytes/ticks, throughput, ACK latency, retransmissions, reordering, peak buffering, interrupt completions, recovery work, frame/notification work, and queue capacity;
- asynchronous ABI 16 UDP transport start, real DNS completion, bounded timeout, and cancellation;
- asynchronous ABI 16 TCP transport start, exact Ring 3 response completion, RST failure, and cancellation;
- compatibility TCP connect, data, and close;
- the exact HTTP/1.1 response reaching Ring 3;
- bounded refused-connection handling;
- network diagnostics readiness;
- a normal long-lived serial terminal at `GENOS_READY`.

It then rebuilds a production kernel without `network-test-faults`, scans that binary to ensure the fault-trigger markers are absent, and boots the modern network configuration without the test server or inbound host forward. That boot requires DHCP, DNS, a clean asynchronous TCP RST failure, `USER_SHELL_READY`, and `GENOS_READY` while forbidding false HTTP and passive-accept success markers. The full `cargo xtask test` includes both network artifacts after storage and terminal validation and leaves the production image as the final build output.

## GenOS 0.55 TCP hardening

The host packet harness in `kernel/src/network_transport_tests.rs` compiles the actual `network.rs` state machines with a memory Ethernet device. The regression matrix covers duplicate delivery, multiple receive gaps and deferred FIN, invalid/guessed reset sequences, challenge ACK rate limiting, forged ACK/window changes, zero-window deferral and bounded exponential probes, small advertised windows, ambiguous RTT samples after retransmission, slow readers, sequence wrap, cancellation, slot exhaustion/reuse, truncations, corrupted checksums, and foreign tuples. This supplements the real VirtIO/QEMU gate; it is not physical-device evidence.

A send remains bound to its request while the peer window is zero. A bounded probe asks for an updated window without advancing queued user bytes. When the window opens, each flight is limited to that advertised size; the unacknowledged suffix remains owned until exact ACK completion. Persist timeout releases the slot without counting window closure as congestion loss. The fixed 128-byte application queue and one outstanding segment remain intentional limits.

Deferred receive data carries an explicit acknowledgment state and cannot reach Ring 3 before every earlier gap closes. Partial overlap with a held segment is declined without overwriting the first accepted bytes. FIN cannot pass a full receive window, and no further data is admitted after EOF. Incoming data cannot postpone an outstanding retransmission timer, and unread application buffers do not prevent timer progress. Retransmitted ACKs do not update RTT estimates (Karn's rule). The existing bounded RTO policy is still not a full RFC 6298 estimator.

Reset validation and send-window update ordering follow the relevant rules in [RFC 9293](https://www.rfc-editor.org/rfc/rfc9293.html); RTT sampling follows [RFC 6298](https://www.rfc-editor.org/rfc/rfc6298.html). General partial ACK recovery, fast retransmit, selective acknowledgments, multiple outbound flights, arbitrary segment merging, and long Internet sessions remain open.

IPv6 control traffic now shares the frame-device path; see [the IPv6 contract](IPV6.md). Both modern network boots require its SLAAC/DAD and exact echo markers while retaining the IPv4 socket proofs.

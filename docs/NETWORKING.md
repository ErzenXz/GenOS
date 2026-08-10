# Networking

GenOS 0.49 combines modern VirtIO transport, asynchronous ABI 16 UDP/TCP clients, and ABI 17 TCP listener authority with one bounded passive request/response stream. QEMU presents a VirtIO 1.x PCI network device with its legacy interface disabled, and the kernel runs DHCP, ICMP, DNS, Ring 3 UDP/TCP client requests, and a host-forwarded inbound handshake, payload exchange, half-close, and orderly close through that device. No host socket shortcut, firmware network stack, guest agent, or Linux component handles guest packets.

This is a modern device foundation with bounded client and single-stream server transactions. It is not yet a production Internet stack; long-lived segmented streams, concurrent TCP service, production congestion/loss behavior, interrupt-driven completion, IPv6, and TLS remain explicit gates below.

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

Every receive descriptor owns a 2048-byte device-writable buffer. Modern VirtIO's 12-byte `virtio_net_hdr`, including `num_buffers`, precedes the Ethernet frame. Incoming lengths and descriptor IDs are validated before copying into the stack-owned 1518-byte frame buffer, after which the receive descriptor is returned to the device. TX prepends a zeroed 12-byte header, pads short Ethernet frames, waits for the used-ring completion with a fixed deadline, and never reuses its DMA buffer while the device owns it.

The current queue path is bounded polling. Interrupt-driven completion, MSI-X routing, multiple queue pairs, and negotiated offloads belong to the performance milestone and cannot be claimed from this implementation.

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
- one passive SYN/SYN-ACK/ACK handshake feeding an exact bound listener, followed by one bounded request, response, peer half-close, and guest FIN.

IPv4 headers and ICMP, UDP, and TCP payloads are checksum validated. IPv4 fragments, invalid header lengths, inconsistent total lengths, malformed UDP lengths, invalid TCP data offsets, and truncated DNS names are rejected. Host tests feed every truncation of a valid frame plus malformed length and header combinations through the parsers.

QEMU's reference user network assigns `10.0.2.15`, gateway `10.0.2.2`, and DNS `10.0.2.3`. On-link destinations are resolved directly with ARP; off-link destinations resolve the gateway. Storage-only smoke phases intentionally omit a NIC, and `NETWORK_DEVICE_UNAVAILABLE` remains non-fatal.

## ABI 15 exchange API

ABI 15 exposes three bounded calls:

- `network_config` copies the current IPv4 configuration and MAC address into validated process memory;
- `udp_exchange` sends one datagram and returns the first matching response from the exact address and port;
- `tcp_exchange` opens one connection, sends one request, collects a bounded response, acknowledges received data, and closes after the peer's FIN.

Every request is copied from a validated caller-owned mapping before the device is touched. Every response is copied back only within the validated output capacity.

`SHELL.ELF` uses the UDP API to construct and send a DNS A query for `example.com`, parses the answer in Ring 3, then uses the TCP API to send an HTTP/1.1 request with a required `Host` header to the deterministic xtask server through QEMU's `10.0.2.2` host alias. It requires `HTTP/1.1 200` and `GENOS_OK` before emitting `USER_HTTP_REQUEST_OK` and `USER_SOCKET_API_READY`. This plaintext endpoint exists only inside the deterministic test network and is not an approved Internet security boundary.

## ABI 16 asynchronous clients and ABI 17 listeners

ABI 16 adds process-owned UDP and TCP-stream objects. `socket_open` returns an opaque generation-safe handle registered as a socket in the caller's unified typed capability table. `socket_connect`, `socket_send`, `socket_receive`, `socket_status`, `socket_shutdown`, and `socket_close` require that exact live capability.

Each process may own four socket handles. Every socket has separate fixed 128-byte send and receive queues. Sends are admitted only while capacity exists; saturation returns `USER_ERROR_WOULD_BLOCK` and never overwrites queued bytes. Receives preserve unread suffix bytes after a partial copy and return `WOULD_BLOCK` when no data is available. Status exposes protocol, lifecycle state, readiness bits, and exact queued-byte counts. Shutdown clears only the selected direction's queued work, close revokes the handle, and process termination reclaims every socket owned by that exact process incarnation.

For UDP, `ProcessManager` moves one admitted datagram into an in-flight slot and assigns a nonzero monotonic request ID. The request records the exact process slot, incarnation, task, PID, socket handle, destination, port, and copied payload. `RuntimeCoordinator` revalidates that full identity before starting or completing transport. One bounded coordinator slot resolves the next hop, sends the datagram, consumes at most one received frame per coordinator tick, validates the response address, port, IPv4 header, and UDP checksum, then places at most 128 response bytes into the owning socket's receive queue. The syscall itself only admits or reads bounded queue data; it does not wait for the network.

The UDP state machine allows three attempts separated by scheduler deadlines. Timeout clears the in-flight request, marks the socket `Failed`, and exposes error readiness. Write shutdown or close invalidates the exact request; the coordinator cancels its packet operation and drops any stale completion. A second transport request cannot overwrite the occupied coordinator slot. These limits are deliberate: per-process socket count, send/receive bytes, in-flight bytes, request copies, response copies, retry count, NIC polls per tick, and concurrent coordinator transports are all fixed.

`SHELL.ELF` sends a real DNS A query through `socket_send`, yields with `sleep`, receives the answer through `socket_receive`, and validates it in Ring 3 before emitting `USER_SOCKET_UDP_ASYNC_READY`. It separately proves bounded timeout and write-shutdown cancellation. The smoke suite requires the start, completion, timeout, cancellation, and current ABI 17 capability markers on both modern VirtIO network boots.

For TCP, the same exact request identity owns one bounded client transaction. The coordinator resolves ARP, validates the exact SYN-ACK acknowledgment, sends ACK plus at most 128 request bytes, and accepts response segments only from the exact IPv4 address and port with valid checksums, acknowledgment, and next sequence. In-order bytes accumulate into a fixed 128-byte response; duplicate or out-of-order segments receive the current cumulative ACK without entering the queue. FIN is acknowledged and followed by an active close. RST, overflow, exhausted retry deadlines, and invalid completion authority mark the socket `Failed`.

`SHELL.ELF` sends an HTTP/1.1 request with `socket_send`, yields while the coordinator progresses TCP, then validates the 65-byte response returned by `socket_receive` before emitting `USER_SOCKET_TCP_ASYNC_READY`. The deterministic host accepts a second connection for the retained ABI 15 compatibility exchange. Without that server, QEMU returns a real RST; the socket exposes error readiness and the operating system continues booting. A separate in-flight write shutdown proves protocol-specific cancellation and stale-completion rejection.

The TCP client path is deliberately one bounded request/response transaction, not a general long-lived byte stream.

ABI 17 adds TCP-only `socket_bind`, `socket_listen`, and non-blocking `socket_accept`. Ports below 1024 are reserved; every admitted local port has one owner across all live process socket sets. A listener owns a fixed backlog of at most two pending peers. Empty accept returns `USER_ERROR_WOULD_BLOCK`; a queued peer makes the listener readable and accept-ready; an accepted peer becomes a fresh generation-safe TCP capability registered in the caller's unified typed handle table. Child allocation failure preserves the pending peer, typed-table registration failure rolls back the child, and close or process cleanup releases the port.

`SHELL.ELF` first proves the ABI 17 authority contract: low-port and oversized-backlog calls are rejected, an altered handle grants nothing, duplicate bind returns unavailable, empty accept returns `WOULD_BLOCK`, a closed listener is stale, and the same port can be rebound after close. It then binds port 18081 for a bounded optional window. The coordinator accepts only an exact checksum-valid SYN for that live destination port, sends SYN-ACK with three-attempt retry and timeout, validates the peer tuple and final sequence/acknowledgment, and queues the established peer through the exact process slot, incarnation, PID, handle, and port. Missing listeners and saturated backlogs receive refusal resets; closing the listener cancels the pending handshake.

The completed handshake's peer MAC, IPv4 address, ports, and initial sequence numbers travel unchanged through backlog admission into the accepted child. The stream engine accepts payload or FIN only with the exact tuple, checksum, next sequence, and valid acknowledgment. At most 128 inbound bytes are copied into the socket queue. A Ring 3 send moves at most 128 bytes into a separate in-flight record with a nonzero request ID bound to the exact process slot, incarnation, task, PID, handle, peer, and byte count; completion waits for the exact cumulative ACK. Oversized input resets the peer, stale authority cancels with RST, and response/FIN retransmission plus idle lifetime are bounded.

The deterministic QEMU phase connects a real host `TcpStream` through QEMU forwarding, sends `GENOS_PING`, half-closes its write side, verifies `GENOS_PONG`, and requires EOF after the guest FIN. Required markers cover SYN admission, handshake, accepted capability, Ring 3 receive, response ACK, peer FIN, guest FIN, and final close. A separate boot without host forwarding must still reach `GENOS_READY`. This is one bounded stream transaction, not general server TCP: simultaneous handshakes, multiple accepted clients, arbitrary segmentation/reassembly, listener wakeup fairness, dynamic windows, congestion control, and long-lived service remain open.

## Loss and timeout policy

DHCP, ARP, UDP response waits, active TCP SYN, passive TCP SYN-ACK, active request data, passive response data, and passive FIN allow three bounded attempts. Compatibility exchanges use a fixed poll budget and return `USER_ERROR_UNAVAILABLE` after exhaustion. Asynchronous paths use scheduler deadlines and bounded per-tick NIC recovery polling, then expose `Failed` plus error readiness instead of leaving a socket in flight. The passive stream also has a fixed idle lifetime so an accepted but inactive or malformed peer cannot monopolize the only transport slot indefinitely. RST fails immediately. QEMU requires refused TCP compatibility, asynchronous TCP RST, ABI 16 UDP timeout, protocol-specific cancellation, and the complete passive request/response/close marker sequence.

## Required modernization milestones

The following are release gates, not optional ideas:

1. **Long-lived accepted TCP streams and concurrency:** add arbitrary bounded segmentation/reassembly, fair wakeups, simultaneous handshakes, multiple accepted streams, cancellation, and cross-client resource budgets to the proven single-transaction path.
2. **Production TCP behavior:** retransmission timers, RTT estimation, dynamic windows, congestion control, out-of-order reassembly, selective acknowledgments where negotiated, wire-level half-close, reset semantics, and loss/reordering tests.
3. **VirtIO performance completion:** interrupt-driven RX/TX, MSI-X, queue recovery, multiple queue pairs where useful, measured batching, and carefully negotiated checksum/segmentation offloads with fallback tests.
4. **IPv6 dual stack:** IPv6 parsing and routing, ICMPv6, neighbor discovery, router advertisements, SLAAC, DNS AAAA, path-MTU handling, and dual-stack policy tests. IPv4 remains supported but cannot be the only production path.
5. **Secure networking:** kernel entropy first; TLS 1.3 and certificate validation in isolated userspace; HTTPS; trust-store/update policy; time validation; and negative tests for expired, mismatched, revoked, malformed, and untrusted certificates. GenOS will not invent its own cryptography.

No application that handles credentials, tokens, personal data, updates, or packages may treat the current plaintext exchange API as an approved transport.

## Verification

`cargo xtask test-network` builds GenOS, starts a deterministic host HTTP/1.1 server, and boots QEMU with modern-only VirtIO PCI networking. It requires:

- VirtIO 1.x feature and split-queue readiness;
- the exact modern driver and transport marker;
- eight-buffer packet ownership readiness;
- DHCP configuration and ICMP echo;
- Ring 3 DNS resolution;
- the ABI 17 Ring 3 socket-capability lifecycle, listener authority, duplicate-bind refusal, empty accept, close/rebind cleanup, and forged/stale denial;
- a real host-forwarded passive SYN/SYN-ACK/ACK exchange, exact backlog admission, and Ring 3 accepted-child capability;
- the exact bounded host request reaching Ring 3, an ACK-confirmed Ring 3 response, peer half-close, guest FIN acknowledgment, and EOF;
- asynchronous ABI 16 UDP transport start, real DNS completion, bounded timeout, and cancellation;
- asynchronous ABI 16 TCP transport start, exact Ring 3 response completion, RST failure, and cancellation;
- compatibility TCP connect, data, and close;
- the exact HTTP/1.1 response reaching Ring 3;
- bounded refused-connection handling;
- network diagnostics readiness;
- a normal long-lived serial terminal at `GENOS_READY`.

It then boots the same modern network configuration without the test server or inbound host forward and requires DHCP, DNS, a clean asynchronous TCP RST failure, `USER_SHELL_READY`, and `GENOS_READY` while forbidding false HTTP and passive-accept success markers. The full `cargo xtask test` includes this network phase after storage and terminal validation.

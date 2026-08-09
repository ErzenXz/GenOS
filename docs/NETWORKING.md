# Networking

GenOS 0.42 completes Stage 5 with one deliberately small QEMU network path. The kernel drives an emulated NE2000 adapter through 16-bit programmed I/O at `0x300`; no Linux, firmware network stack, host socket shortcut, or QEMU guest agent handles guest packets.

## Packet ownership and device model

The driver owns one bounded 1518-byte receive buffer with three explicit states: free, driver, and stack. NE2000 ring data moves into the buffer as driver-owned bytes, transfers to the stack only after the hardware header and length are validated, and returns to free immediately after parsing. Protocol values that outlive a receive iteration copy their bytes into owned state.

Transmit data is assembled in bounded stack buffers and copied into the NE2000 transmit page with remote DMA I/O. Receive-ring pages, wraparound, next-page pointers, hardware lengths, and transmit completion all have explicit bounds or polling deadlines.

## Protocol stack

The initial stack implements:

- Ethernet II framing;
- ARP requests and replies for a destination or its configured gateway;
- IPv4 without fragmentation or reassembly;
- ICMP echo request/reply;
- UDP datagrams;
- DHCP discover, offer, request, and acknowledgment;
- DNS queries over UDP;
- one active-open TCP exchange state machine with SYN, SYN-ACK, ACK, data, FIN, and RST handling.

IPv4 headers and ICMP, UDP, and TCP payloads are checksum validated. IPv4 fragments, invalid header lengths, inconsistent total lengths, malformed UDP lengths, invalid TCP data offsets, and truncated DNS names are rejected. Host tests feed every truncation of a valid frame plus malformed length and header combinations through the parsers.

## Configuration and routing

When the NE2000 device exists, boot sends DHCP discover and request messages and records the acknowledged address, subnet, gateway, and DNS server. QEMU's reference user network assigns `10.0.2.15`, gateway `10.0.2.2`, and DNS `10.0.2.3`. On-link destinations are resolved directly with ARP; off-link destinations resolve the gateway.

The storage-only QEMU phases intentionally omit a NIC. `NETWORK_DEVICE_UNAVAILABLE` is non-fatal, and the server terminal, filesystem, and process runtime continue normally.

The normal `cargo xtask run` path includes the NE2000 device. The deterministic HTTP server exists only during tests; if it is absent, the bounded diagnostic exchange returns unavailable and the Ring 3 shell still reaches `GENOS_READY`. A separate no-server QEMU phase enforces that graceful-degradation contract.

## ABI 15 exchange API

ABI 15 exposes three bounded calls:

- `network_config` copies the current IPv4 configuration and MAC address into validated process memory;
- `udp_exchange` sends one datagram and returns the first matching response from the exact address and port;
- `tcp_exchange` opens one connection, sends one request, collects a bounded response, acknowledges received data, and closes after the peer's FIN.

Every request is copied from a validated caller-owned mapping before the device is touched. Every response is copied back only within the validated output capacity. The initial API is intentionally an exchange API, not a general asynchronous BSD/POSIX socket compatibility layer. It proves a safe userspace network boundary while persistent socket handles, listening sockets, streaming backpressure, congestion control, and larger windows remain later API-growth work.

`SHELL.ELF` uses the UDP API to construct and send a DNS A query for `example.com`, parses the answer in Ring 3, then uses the TCP API to request `/` from the deterministic xtask HTTP server through QEMU's `10.0.2.2` host alias. It requires `HTTP/1.0 200` and `GENOS_OK` before emitting `USER_HTTP_REQUEST_OK` and `USER_SOCKET_API_READY`. The `net` command reports whether this boot-time diagnostic path is available.

## Loss and timeout policy

DHCP, ARP, UDP response waits, TCP SYN, and the initial TCP data request allow three bounded attempts. Each device or protocol wait has a fixed poll budget and returns `USER_ERROR_UNAVAILABLE` after exhaustion. RST also closes an exchange as unavailable. The QEMU proof connects to a refused host port and requires `USER_NETWORK_TIMEOUT_OK`; it never waits indefinitely or leaves a process blocked after failure.

This is a correctness baseline, not a claim of mature Internet TCP. The current stack does not implement IP fragmentation, IPv6, TCP listening, selective acknowledgments, congestion control, dynamic receive windows, or out-of-order reassembly.

## Verification

`cargo xtask test-network` builds GenOS, starts a deterministic host HTTP server, boots QEMU with NE2000 and user networking, and requires:

- device and packet-ownership readiness;
- DHCP configuration;
- ICMP echo;
- Ring 3 DNS resolution;
- TCP connect, data, and close;
- the exact HTTP response reaching Ring 3;
- bounded refused-connection handling;
- the network diagnostics command becoming ready;
- a normal long-lived serial terminal at `GENOS_READY`.

It then boots the same network configuration without the test HTTP server and requires DHCP, DNS, `USER_SHELL_READY`, and `GENOS_READY` while forbidding a false `USER_HTTP_REQUEST_OK` marker.

The full `cargo xtask test` includes the same network phase after all storage and terminal phases.

# IPv6 host foundation

GenOS 0.55 starts Stage 5.5 with a bounded IPv6 host control path. Native QEMU tests prove a global/ULA address derived from an advertised prefix, duplicate-address detection (DAD) for both link-local and global addresses, and an ICMPv6 echo to the discovered router over modern VirtIO. No guest IPv6 address, router IPv6 address, or prefix is hard-coded.

The production socket ABI remains IPv4-only. IPv6 UDP/TCP applications, DNS AAAA resolution, IPv6-only boot, address selection, and general Internet operation are not implemented by this milestone.

## Ownership and timing

`kernel::ipv6::Host` owns bounded configuration state and creates at most one control frame per poll. It allocates no heap memory and accesses no hardware. `NetworkStack` handles transmission and routes IPv6 frames before the existing IPv4 consumers can discard them. `RuntimeCoordinator` advances control timers using the hardware 100 Hz clock, including during boot probes that use synthetic scheduler ticks for other work.

The interface performs one DAD solicitation with a one-second wait for each tentative address, then up to three router solicitations four seconds apart. A conflict leaves the interface in `Duplicate` with no usable route. Router and address lifetimes use saturating monotonic deadlines; expiration clears the route and restarts solicitation. Discovery failure is bounded and cannot block the serial terminal. The bootstrap currently starts when the IPv4 reference interface is available.

Link-local addresses and the interface half of SLAAC addresses use modified EUI-64 derived from the NIC MAC. This deterministic VM policy exposes a stable interface identifier; privacy addresses and randomized interface identifiers require the later entropy/privacy work.

## Validation policy

- Ethernet II IPv6 frames are bounded to 1,518 bytes. Version, nonzero hop limit, payload length, source address class, destination, and truncation are checked before access.
- At most one Hop-by-Hop header followed by one Destination Options header is accepted, with padding options only. Duplicate or reordered extension headers, unknown options, fragmentation (including atomic fragments), routing headers, AH/ESP, and jumbograms fail closed. Unsupported features are discarded, not interpreted as transport bytes.
- ICMPv6, UDP, and TCP checksum helpers use the IPv6 pseudo-header. UDP checksum omission is rejected. The pure transport validator does not imply a wire-backed IPv6 socket implementation.
- Router advertisements must be checksum-valid, have hop limit 255 and code zero, originate from a link-local router, and target this interface or all nodes. All option lengths are checked, including unknown options. A source link-layer option must match the Ethernet source. The bootstrap requires a /64 prefix with both autonomous and on-link flags, positive valid/preferred lifetimes, and preferred lifetime no greater than valid lifetime.
- An advertised MTU below 1,280 is ignored. Otherwise it is bounded to the 1,500-byte Ethernet MTU; the default is 1,280. Echo packets stay below the minimum MTU. There is no general path-MTU cache or Packet Too Big recovery yet.
- Neighbor solicitations and advertisements require correct hop limit, code, target class, lengths, link-layer options, and destination. DAD solicitations cannot include a source link-layer option. A solicited advertisement addressed to multicast is rejected.
- Neighbor solicitations for usable local addresses receive checksum-valid advertisements. A router echo reply must match its source, destination, identifier, sequence, and exact payload before the success marker is emitted. Bounded unicast echo requests can be answered without userspace involvement.

The implementation follows the relevant bounded parts of [IPv6](https://www.rfc-editor.org/rfc/rfc8200.html), [Neighbor Discovery](https://www.rfc-editor.org/rfc/rfc4861.html), [SLAAC](https://www.rfc-editor.org/rfc/rfc4862.html), and [ICMPv6](https://www.rfc-editor.org/rfc/rfc4443.html). It does not claim complete conformance to those standards. Neighbor Discovery remains unauthenticated on the local link; a valid advertisement is network configuration, not trusted identity or application authority.

## Evidence and remaining work

`cargo test -p kernel --lib` exercises actual wire bytes: every RA truncation, checksum corruption, malformed options/extensions, invalid hop limits and lifetimes, duplicate addresses, invalid multicast advertisements, neighbor replies, exact echo matching, expiration, mandatory UDP checksums, and output bounds.

`cargo xtask test-network` requires `IPV6_SLAAC_READY prefix=ra dad=passed` and `IPV6_ICMP_ECHO_OK` in both fault-enabled and production network boots. The existing IPv4 DNS, socket, TCP, and timeout proofs remain required. Serial logs are `build/serial-network.log` and `build/serial-network-normal-run.log`.

Still open: IPv6-only initialization, UDP/TCP socket addressing and demultiplexing, DNS AAAA and RDNSS, dual-stack bind/connect policy, route renewal and multi-router selection while a route is live, neighbor cache/reachability state, MLD, DHCPv6, privacy addresses, comprehensive ICMPv6 error handling, PMTU, and physical-network validation. TLS 1.3 and authenticated application traffic belong to Stage 6.

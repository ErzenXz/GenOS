# TCP requirement-to-test matrix

N1.1 records the current experimental IPv4 TCP profile and its gaps. It does
not promote GenOS or close the TCP implementation/qualification items N1.2–N1.5.

Profile: `genos-tcp-ipv4-bounded-v1`. Source review: 2026-10-13.

This document is generated from [the reviewed matrix](../tools/tcp-requirements.json).
`python3 tools/check_tcp_matrix.py` checks all 119 RFC 9293 Appendix B identifiers,
all 25 topics, production source hashes, compiled host test names, guest
marker producers/consumers and the required CI context. It also rejects a
stale generated document. Source/test changes require review and receipt updates;
`--write-doc` renders the reviewed data and does not refresh hashes or statuses.

## Scope and interpretation

The wire path is Ethernet/IPv4 TCP. Active transactions have one 128-byte
request/response; passive streams have four global handshake/stream slots,
two per process incarnation, 128-byte application buffers, one outstanding
send flight and one deferred receive segment. Repeated passive operations
can transfer more bytes than a buffer. Three retry attempts and 200 logical
ticks of passive idle lifetime are resource policies. Logical ticks are not
a qualified wall-clock RFC timer. IPv6 application TCP remains N2.1.

`verified` means only the row's stated behavior has positive/negative tests.
`partial` and `gap` retain missing mandatory behavior; `unsupported` identifies
unadvertised features and their conditional obligations. No status asserts
whole-RFC conformance. Empty test cells are explicit gaps, never passing tests.

Pinned sources:

- [RFC 5681](https://www.rfc-editor.org/rfc/rfc5681.html), published 2009-09.
- [RFC 5961](https://www.rfc-editor.org/rfc/rfc5961.html), published 2010-08.
- [RFC 6298](https://www.rfc-editor.org/rfc/rfc6298.html), published 2011-06.
- [RFC 9293](https://www.rfc-editor.org/rfc/rfc9293.html), published 2022-08.

RFC 9293 Appendix B supplies the identifier inventory; RFC 6298 and
RFC 5681 supply the companion timer/congestion review, and RFC 5961
supplies additional reset/injection review. The table below describes
GenOS behavior from its code/tests, not copied normative prose.

## Input classification

| Class | Current meaning |
| --- | --- |
| `malformed` | Bad frame/checksum/header/option extent; drop before TCP state mutation. |
| `state-inapplicable` | Well-formed segment for another tuple or invalid sequence/ACK in this state; ignore or ACK/challenge by state. |
| `valid-unsupported` | Well-formed optional offer or feature with no advertised semantics; structurally admit/ignore or decline without calling the format malformed. |
| `profile-limit` | Valid traffic outside fixed segment/queue/owner/lifetime budgets; defined backpressure/reset/failure, not format corruption. |

A legal larger segment, zero window, urgent indication or optional offer
is not malformed solely because this bounded implementation lacks its
complete semantics. Mandatory missing behavior stays a conformance gap.

## Requirement and implementation inventory

| Topic / status | RFC identifiers and sources | Current behavior / owner |
| --- | --- | --- |
| header: Header and payload admission **verified** | <br>[RFC 9293 §3.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.1) | The data offset bounds a 20..60-byte header; payload starts after that header. Reserved received bits are ignored. No option walk exceeds 40 bytes.<br>[parse_tcp](../kernel/src/net.rs) |
| checksum: IPv4 TCP checksum **verified** | `MUST-2`, `MUST-3`<br>[RFC 9293 §appendix-B](https://www.rfc-editor.org/rfc/rfc9293.html#appendix-B) | The production sender checksums the IPv4 pseudo-header and segment. Both passive and active receive paths validate that checksum before tuple/state handling.<br>[send_tcp_with_window](../kernel/src/network.rs); [decode_tcp_reply](../kernel/src/network.rs); [decode_passive_segment](../kernel/src/network.rs) |
| options: Option structure and unknown kinds **verified** | `MUST-5`, `MUST-6`, `MUST-7`, `MUST-64`, `MUST-68`, `MUST-69`<br>[RFC 9293 §appendix-B](https://www.rfc-editor.org/rfc/rfc9293.html#appendix-B) | EOL/NOP, option extents and MSS shape are checked at byte alignment. Unknown well-formed length-bearing kinds are skipped; bad lengths and nonzero EOL padding are dropped before stream mutation.<br>[tcp_options_valid](../kernel/src/net.rs) |
| mss: Mandatory MSS and segmentation **gap** | `MUST-4`, `MUST-14`, `MUST-15`, `MUST-16`, `MUST-65`, `MUST-67`, `SHLD-5`, `SHLD-6`, `MAY-3`<br>[RFC 9293 §3.7.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.7.1) | GenOS emits a 20-byte TCP header and limits passive application sends to 128 bytes. Valid received MSS options pass structure checks but are not negotiated.<br>[send_tcp_with_window](../kernel/src/network.rs); [transmit_stream_flight](../kernel/src/network.rs) |
| isn: Initial sequence and reincarnation **gap** | `MUST-8`, `MUST-9`, `SHLD-1`<br>[RFC 9293 §3.4.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.4.1); [RFC 9293 §3.4.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.4.2); [RFC 9293 §3.4.3](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.4.3) | Passive ISNs are deterministic tuple/peer-sequence XOR values. Local sequence arithmetic uses wrapping u32; it is not a secure ISN generator.<br>[start_tcp_passive](../kernel/src/network.rs); [start_tcp_async](../kernel/src/network.rs) |
| open: Connection establishment and passive ownership **partial** | `MUST-10`, `MUST-11`, `MUST-41`, `MUST-42`, `MUST-44`, `MUST-45`<br>[RFC 9293 §3.5](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.5); [RFC 9293 §3.10.7.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.10.7.2); [RFC 9293 §3.10.7.3](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.10.7.3) | Active open and four tuple-keyed passive handshake cells exist; handshake ACKs require exact sequence and acknowledgment. Source address is the configured IPv4 address.<br>[poll_tcp_async](../kernel/src/network.rs); [handle_handshake_segment](../kernel/src/network.rs); [start_tcp_passive](../kernel/src/network.rs) |
| address: Address and peer admission **gap** | `MUST-46`, `MUST-57`, `MUST-63`<br>[RFC 9293 §appendix-B](https://www.rfc-editor.org/rfc/rfc9293.html#appendix-B) | Inbound packets must name the configured destination and exact peer tuple. Socket connect validates zero addresses/ports; full broadcast/multicast/source-address policy is unqualified.<br>[decode_tcp_reply](../kernel/src/network.rs); [decode_passive_segment](../kernel/src/network.rs) |
| close: Close, half-close and TIME-WAIT **partial** | `MUST-12`, `MUST-13`, `SHLD-2`, `SHLD-3`, `SHLD-4`, `MAY-1`, `MAY-2`<br>[RFC 9293 §3.6](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.6); [RFC 9293 §3.6.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.6.1) | Passive peer EOF and guest FIN/ACK have bounded state; post-EOF bytes and duplicate FIN do not redeliver. Reset/failure is reported and slots can be cancelled.<br>[handle_stream_segment](../kernel/src/network.rs); [start_tcp_passive_stream_close](../kernel/src/network.rs); [finish_tcp_passive_stream](../kernel/src/network.rs) |
| rto: Retransmission timing and Karn **partial** | `MUST-18`, `MUST-19`, `MAY-4`<br>[RFC 9293 §3.8.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.1); [RFC 6298 §2](https://www.rfc-editor.org/rfc/rfc6298.html#section-2); [RFC 6298 §3](https://www.rfc-editor.org/rfc/rfc6298.html#section-3); [RFC 6298 §5](https://www.rfc-editor.org/rfc/rfc6298.html#section-5) | Passive sends skip ambiguous RTT samples after retransmission. RTO starts at 25 ticks, uses a bounded heuristic, backs off up to 100 ticks and fails after three attempts.<br>[handle_stream_segment](../kernel/src/network.rs); [poll_tcp_passive_stream](../kernel/src/network.rs) |
| failure: Retry, idle and application failure **partial** | `MUST-20`, `MUST-21`, `MUST-22`, `MUST-23`, `SHLD-9`, `SHLD-10`, `SHLD-11`<br>[RFC 9293 §3.8.3](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.3) | Passive retry and idle lifetimes are finite; active compatibility paths use finite polls. Failed transport removes its slot and socket errors remain observable.<br>[poll_tcp_async](../kernel/src/network.rs); [poll_tcp_passive](../kernel/src/network.rs); [poll_tcp_passive_stream](../kernel/src/network.rs) |
| keepalive: Optional keep-alive facility **unsupported** | `MAY-5`, `MAY-6`, `MUST-24`, `MUST-25`, `MUST-26`, `MUST-27`, `MUST-28`, `MUST-29`, `SHLD-12`<br>[RFC 9293 §3.8.4](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.4) | No keep-alive socket facility is advertised. The 200-tick passive idle deadline is a resource limit, not a keep-alive timer.<br>[refresh_stream_idle](../kernel/src/network.rs) |
| urgent: Urgent-data semantics **gap** | `MUST-30`, `MUST-31`, `MUST-32`, `MUST-33`, `MUST-62`, `SHLD-13`<br>[RFC 9293 §3.8.5](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.5) | The parser retains flags but not urgent-pointer semantics. There is no urgent-data delivery or application notification facility.<br>[parse_tcp](../kernel/src/net.rs) |
| window: Send/receive windows and updates **partial** | `MUST-1`, `MUST-34`, `MUST-66`, `REC-1`, `SHLD-14`, `SHLD-15`, `SHLD-16`, `SHLD-17`, `MAY-7`<br>[RFC 9293 §3.8.6](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.6); [RFC 5681 §3.1](https://www.rfc-editor.org/rfc/rfc5681.html#section-3.1) | Unsigned u16 windows cap one send flight; exact sequence/ACK window updates cannot complete another flight. Receive occupancy advertises zero and reopens after consumption.<br>[handle_stream_segment](../kernel/src/network.rs); [receive_window](../kernel/src/network.rs); [transmit_stream_flight](../kernel/src/network.rs) |
| persist: Zero-window probing and resource lifetime **partial** | `MUST-35`, `MUST-36`, `MUST-37`, `SHLD-29`, `SHLD-30`, `MAY-8`<br>[RFC 9293 §3.8.6.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.6.1) | Queued bytes stay owned while a peer window is zero. Bounded exponential probes do not advance data or count as congestion loss; the idle deadline eventually fails and releases the stream.<br>[poll_tcp_passive_stream](../kernel/src/network.rs) |
| sws: Silly-window avoidance and Nagle **gap** | `MUST-17`, `MUST-38`, `MUST-39`, `SHLD-7`<br>[RFC 9293 §3.7.4](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.7.4); [RFC 9293 §3.8.6.2.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.6.2.1); [RFC 9293 §3.8.6.2.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.6.2.2) | Each bounded queued send can transmit immediately when no flight exists; small windows split that queue. No Nagle/SWS algorithm or disable option is advertised.<br>[transmit_stream_flight](../kernel/src/network.rs); [consume_tcp_passive_stream_receive](../kernel/src/network.rs) |
| ack: ACKs, duplicate data and bounded reassembly **partial** | `MUST-40`, `MUST-58`, `MUST-59`, `SHLD-18`, `SHLD-19`, `SHLD-31`, `MAY-13`<br>[RFC 9293 §3.7](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.7); [RFC 9293 §3.8.6.3](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.6.3); [RFC 5681 §4.2](https://www.rfc-editor.org/rfc/rfc5681.html#section-4.2) | Cumulative ACKs accompany admitted/repeated data; one deferred 128-byte segment preserves a gap and FIN. Conflicting partial overlaps keep the original bytes; reads never expose a gap.<br>[handle_stream_segment](../kernel/src/network.rs); [consume_tcp_passive_stream_receive](../kernel/src/network.rs) |
| push: PUSH and application send policy **partial** | `MUST-60`, `MUST-61`, `SHLD-27`, `SHLD-28`, `MAY-15`, `MAY-16`, `MAY-17`<br>[RFC 9293 §appendix-B](https://www.rfc-editor.org/rfc/rfc9293.html#appendix-B) | Data flights set PSH/ACK and exact ACK completion retires the queued send. The fixed socket API does not expose separate PUSH or FLUSH controls.<br>[transmit_stream_flight](../kernel/src/network.rs) |
| api: Application interface, errors and Diffserv **partial** | `MUST-43`, `MUST-47`, `MUST-48`, `SHLD-20`, `SHLD-21`, `SHLD-22`, `SHLD-23`, `MAY-9`, `MAY-14`<br>[RFC 9293 §3.9.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.9.1) | Owned handles expose send/receive, readiness, shutdown, close and failure. Requests retain exact owner/handle/incarnation identity; partial reads and backpressure are bounded.<br>[receive](../kernel/src/socket.rs); [shutdown](../kernel/src/socket.rs); [complete_transport](../kernel/src/socket.rs) |
| ttl: IP TTL policy **gap** | `MUST-49`<br>[RFC 9293 §3.9.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.9.2) | IPv4 emission uses a fixed TTL of 64.<br>[send_ipv4_from](../kernel/src/network.rs) |
| ip-options: IPv4 option and routing interface **gap** | `MUST-50`, `MUST-51`, `MUST-52`, `MUST-53`, `SHLD-24`, `MAY-10`, `MAY-11`<br>[RFC 9293 §3.9.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.9.2) | IPv4 admission bounds IHL and validates its checksum; TCP option checks do not implement IPv4 routing options.<br>[parse_ipv4_frame](../kernel/src/net.rs) |
| icmp: ICMP error delivery **gap** | `MUST-54`, `MUST-55`, `MUST-56`, `SHLD-25`, `SHLD-26`<br>[RFC 9293 §3.9.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.9.2); [RFC 9293 §3.8.3](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.3) | The current coordinator handles echo/control traffic; TCP has no complete typed ICMP error path.<br>[receive_ipv4](../kernel/src/network.rs) |
| ecn: ECN and larger negotiated extensions **unsupported** | `SHLD-8`<br>[RFC 9293 §3.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.1); [RFC 9293 §3.2.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.2.1) | ECN negotiation, window scale, timestamps, SACK and multipath are not advertised; structurally valid unknown TCP options are ignored.<br>[parse_tcp](../kernel/src/net.rs) |
| reset: Reset/challenge ACK and injection defense **partial** | `MAY-12`<br>[RFC 9293 §3.5.3](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.5.3); [RFC 5961 §3](https://www.rfc-editor.org/rfc/rfc5961.html#section-3); [RFC 5961 §4](https://www.rfc-editor.org/rfc/rfc5961.html#section-4); [RFC 5961 §5](https://www.rfc-editor.org/rfc/rfc5961.html#section-5); [RFC 5961 §7](https://www.rfc-editor.org/rfc/rfc5961.html#section-7) | An exact receive-sequence RST resets the passive stream; an in-window guess is challenged at most once per tick; foreign/out-of-window data cannot complete a flight.<br>[handle_stream_segment](../kernel/src/network.rs); [handle_handshake_segment](../kernel/src/network.rs) |
| congestion: Congestion control and alternate algorithms **partial** | `MAY-18`<br>[RFC 9293 §3.8.2](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.8.2); [RFC 5681 §3.1](https://www.rfc-editor.org/rfc/rfc5681.html#section-3.1); [RFC 5681 §3.2](https://www.rfc-editor.org/rfc/rfc5681.html#section-3.2); [RFC 5681 §4.1](https://www.rfc-editor.org/rfc/rfc5681.html#section-4.1); [RFC 5681 §4.3](https://www.rfc-editor.org/rfc/rfc5681.html#section-4.3) | Passive timeout lowers the threshold and sets cwnd to one 128-byte MSS; exact ACK grows a capped cwnd. Each flight is bounded by both peer window and cwnd.<br>[handle_stream_segment](../kernel/src/network.rs); [poll_tcp_passive_stream](../kernel/src/network.rs); [transmit_stream_flight](../kernel/src/network.rs) |
| capacity: Bounded ownership, cancellation and concurrency **partial** | <br>[RFC 9293 §3.9.1](https://www.rfc-editor.org/rfc/rfc9293.html#section-3.9.1); [RFC 9293 §7](https://www.rfc-editor.org/rfc/rfc9293.html#section-7) | Four global handshakes/streams and two per incarnation bound authority. Host tests cover exact request completion, sibling cancellation, exhaustion and slot reuse.<br>[cancel_tcp_passive_stream](../kernel/src/network.rs); [start_tcp_passive_stream](../kernel/src/network.rs) |

## Executable evidence and remaining scenarios

Host evidence executes the production parser/state machine with a memory
frame device or the production socket authority model. It does not replace
privilege, DMA or real device tests. Names below must appear in the compiled
`cargo test -p kernel --lib -- --list` output.

| Topic | Host tests and outcomes | Missing behavior / next scenarios |
| --- | --- | --- |
| header | `net::tests::malformed_packets_are_rejected_without_indexing_past_bounds` (positive, negative)<br>`net::tests::tcp_options_admit_unknown_and_unaligned_options_without_eating_payload` (positive, negative) | The stated bounded behavior is covered; no broader inference. |
| checksum | `network_transport::network::tests::checksum_corruption_and_foreign_tuple_never_change_stream_state` (positive, negative)<br>`net::tests::passive_tcp_handshake_requires_exact_bounded_packets` (positive, negative) | The stated bounded behavior is covered; no broader inference. |
| options | `net::tests::tcp_options_admit_unknown_and_unaligned_options_without_eating_payload` (positive, negative)<br>`net::tests::tcp_options_reject_bad_lengths_known_mss_shape_and_nonzero_eol_padding` (negative)<br>`network_transport::network::tests::malformed_tcp_options_cannot_mutate_a_stream_but_unknown_valid_options_deliver` (positive, negative) | The stated bounded behavior is covered; no broader inference. |
| mss | `net::tests::tcp_options_admit_unknown_and_unaligned_options_without_eating_payload` (positive) | No advertised/negotiated MSS, path-dependent effective MSS or IPv6 TCP MSS. Structural MSS acceptance does not satisfy negotiation requirements. Test absent and small peer MSS on SYN, MSS emission, non-SYN options and payload-size bounds. |
| isn | `network_transport::network::tests::sequence_wrap_preserves_order_and_exact_send_completion` (positive) | No clock/secret PRF ISN component, quiet-time or old-incarnation segment exclusion; entropy and long-session qualification remain open. Rapidly reuse the same tuple after close/reboot and inject retained segments; test a reviewed ISN generator. |
| open | `net::tests::passive_tcp_handshake_requires_exact_bounded_packets` (positive, negative)<br>`socket::tests::listener_backlog_and_accepted_children_are_bounded_capabilities` (positive, negative, cleanup) | No simultaneous open or complete RFC state transition/previous-state model. Host predicate tests and the guest handshake do not cover every handshake anomaly. Script duplicate SYN/SYN-ACK, simultaneous open, early data/FIN and per-owner listener interference. |
| address | `network_transport::network::tests::checksum_corruption_and_foreign_tuple_never_change_stream_state` (positive, negative)<br>`socket::tests::tcp_transport_preserves_connecting_state_until_exact_completion` (positive, negative, cleanup) | No complete invalid/broadcast/multicast OPEN/SYN campaign or general source-address selection. Exercise broadcast, multicast, zero and forbidden source/destination addresses at socket and wire boundaries. |
| close | `network_transport::network::tests::no_data_or_second_fin_is_admitted_after_peer_eof` (positive, negative)<br>`network_transport::network::tests::fin_waits_for_an_open_receive_window` (positive, negative)<br>`network_transport::network::tests::cancellation_and_reset_leave_sibling_stream_usable` (positive, negative, cleanup) | No 2MSL TIME-WAIT, complete simultaneous close/half-open recovery or long-lived tuple-retirement proof. Script both close directions, simultaneous close, lost final ACK and delayed old packets after slot/tuple reuse. |
| rto | `network_transport::network::tests::retransmitted_ack_does_not_supply_an_ambiguous_rtt_sample` (positive, negative)<br>`network_transport::network::tests::slow_reader_does_not_stop_outbound_retransmissions` (positive, negative) | No SRTT/RTTVAR estimator, RFC second-scale floor/ceiling or complete per-state timer management. This bounded VM heuristic is not RFC 6298 conformance. Use a controllable clock for initial/first/subsequent RTT, Karn, earliest expiry, timer restart and repeated loss. |
| failure | `network_transport::network::tests::zero_window_persists_with_bounded_probes_and_releases_its_slot` (positive, negative, cleanup)<br>`socket::tests::capacity_is_hard_bounded_and_failed_sockets_are_observable` (positive, negative, cleanup) | No configurable R1/R2 or three-minute SYN retry policy, negative route advice, or standardized soft/hard ICMP error report. Test independent R1/R2 thresholds, user abort, SYN persistence and failure/report cleanup. |
| keepalive | No executable claim | Keep-alive configuration and its conditional requirements have no implementation/test claim. Well-formed keep-alive probes are not malformed merely because the API is absent. If enabled, test default-off, configurable idle interval, probe shape and lost ACK tolerance. |
| urgent | No executable claim | Mandatory urgent handling is missing; a valid URG segment is not classified as malformed. Absence is a conformance gap, not a satisfied optional exclusion. Test pointer interpretation, zero-window urgent handling, notification and queued-byte observation. |
| window | `network_transport::network::tests::forged_ack_cannot_change_peer_window_or_complete_send` (positive, negative)<br>`network_transport::network::tests::full_receive_window_reopens_after_consumption_without_losing_bytes` (positive, negative)<br>`network_transport::network::tests::zero_window_defers_payload_and_reopening_resumes_exact_bytes` (positive, negative) | No general window shrink/partial ACK/multiple-flight model, widened window scaling or urgent-at-zero handling. Test window shrink around SND.UNA, partial ACK, reordered updates and RST/URG while zero. |
| persist | `network_transport::network::tests::zero_window_defers_payload_and_reopening_resumes_exact_bytes` (positive, negative)<br>`network_transport::network::tests::zero_window_persists_with_bounded_probes_and_releases_its_slot` (positive, negative, cleanup) | The fixed idle limit does not satisfy indefinite acknowledged zero-window persistence. No general persist lifetime or application-controlled resource-abort qualification. Separate indefinite valid zero-window behavior from an explicit user/resource abort and verify cleanup. |
| sws | `network_transport::network::tests::zero_window_defers_payload_and_reopening_resumes_exact_bytes` (positive) | Small-window progress is not sender/receiver SWS or Nagle conformance. Test tiny writes, tiny receive increments, algorithm thresholds and explicit Nagle disable. |
| ack | `network_transport::network::tests::duplicates_are_acknowledged_without_duplicate_delivery` (positive, negative)<br>`network_transport::network::tests::deferred_data_and_fin_wait_until_every_gap_closes` (positive, negative)<br>`network_transport::network::tests::overlap_cannot_deliver_conflicting_deferred_bytes` (positive, negative) | No general multi-slot reassembly/merging, full ACK aggregation or delayed-ACK policy. The one-slot capacity is a profile limit, not malformed input. Test multiple gaps, legal larger segments, arbitrary overlap partitions and a bounded delayed-ACK timer. |
| push | `socket::tests::bounded_queues_report_readiness_and_backpressure` (positive, negative, cleanup)<br>`network_transport::network::tests::zero_window_defers_payload_and_reopening_resumes_exact_bytes` (positive, negative) | No successive-PUSH coalescing or general send-max-segment policy/qualification. Test successive writes/PUSH boundaries, no indefinite buffering, and partial application send completion. |
| api | `socket::tests::receive_is_partial_bounded_and_lossless` (positive, negative, cleanup)<br>`socket::tests::handles_are_generation_safe_and_owner_scoped` (positive, negative, cleanup)<br>`socket::tests::shutdown_cancels_queued_work_and_owner_exit_reclaims_everything` (positive, negative, cleanup) | No local-address OPEN parameter, Diffserv controls or full RFC error-report/FLUSH interface. Test defined local-address binding, error/report control and any future Diffserv/FLUSH options. |
| ttl | No executable claim | No configurable TCP/application TTL interface or corresponding test. Test explicit TTL configuration and emitted header values. |
| ip-options | `net::tests::malformed_packets_are_rejected_without_indexing_past_bounds` (negative) | No source-route interface/return-route handling or IP option negotiation; structurally valid IP options are distinct from malformed IHL. Test valid unknown IP options separately from bad lengths/checksum, and any explicitly supported routing option. |
| icmp | No executable claim | No reviewed hard/soft ICMP error reporting, quoted-tuple validation or route advice. Source quench and other valid ICMP messages are not blanket malformed TCP. Inject correct/forged quoted tuples and verify non-aborting soft errors versus hard error policy. |
| ecn | `net::tests::tcp_options_admit_unknown_and_unaligned_options_without_eating_payload` (positive, negative) | No negotiated extension or related recovery/anti-wrap claim. Unsupported valid input must not become an invalid-format label. Test extension negotiation/decline and ordinary traffic after a valid unsupported offer. |
| reset | `network_transport::network::tests::reset_requires_exact_receive_sequence_and_challenges_in_window_guess` (positive, negative)<br>`network_transport::network::tests::forged_ack_cannot_change_peer_window_or_complete_send` (positive, negative) | The tests qualify bounded passive behavior, not all RFC 5961 states, global challenge budgets, secure ISNs or authenticated transport. Script SYN-RECEIVED/active-open RSTs, repeated same-tick challenge attempts and cross-owner attack budgets. |
| congestion | `network_transport::network::tests::retransmitted_ack_does_not_supply_an_ambiguous_rtt_sample` (positive, negative)<br>`network_transport::network::tests::zero_window_defers_payload_and_reopening_resumes_exact_bytes` (positive, negative) | No full RFC 5681 algorithm qualification, multi-flight accounting, fast retransmit/recovery, byte-counted partial-ACK growth or idle restart. Named AIMD markers are not a conformance result. Use deterministic loss/duplicate-ACK/idle schedules with cwnd/ssthresh/flight invariants and baseline measurements. |
| capacity | `network_transport::network::tests::cancellation_and_reset_leave_sibling_stream_usable` (positive, negative, cleanup)<br>`network_transport::network::tests::stream_capacity_exhaustion_is_bounded_and_reusable` (positive, negative, exhaustion, cleanup)<br>`socket::tests::two_accepted_children_are_served_concurrently_and_independently` (positive, negative, cleanup) | Two peers on one guest listener do not prove wire fairness across distinct process owners, reset quiescence or long-lived sessions. Run different process owners with one stalled/malicious peer, delayed completion and cancellation during device failure. |

## Guest layer and execution

Required CI context: **Build and boot**; command `cargo xtask test-network`.

Actual VirtIO/guest/host TCP exchange, two clients on one listener, selected loss/reorder/FIN and frozen budgets; not all RFC behavior.

Required producer/validator marker bindings:

- `USER_SOCKET_TCP_ASYNC_READY`
- `USER_HTTP_REQUEST_OK`
- `TCP_PASSIVE_HANDSHAKE_OK`
- `TCP_FAULT_DATA_DROP_INJECTED`
- `TCP_FAULT_REORDER_RELEASED`
- `USER_SOCKET_PASSIVE_CONCURRENT_READY streams=2`
- `NETWORK_REGRESSION_BUDGET_OK`

CI runs the compiled host suite and this inventory guard, then the existing
Build and boot suite executes both the fault-injected network phase and
the production phase without host services. Run identity, source/image
hashes, serial and environment metadata stay in the existing evidence
artifacts. Configured wiring is not execution evidence; see the dated
[verification record](VERIFICATION.md).

N1.2/N1.3/N1.4/N1.5 and SEC2 remain open; verified means only the exact declared behavior/tests, never whole-RFC conformance.

## Maintenance

For a TCP source change, review the affected rows and test outcomes, update
`production_sha256` from the final source, record missing/new scenarios,
then render with `--write-doc` and run the guard. Keep the full identifier
inventory even for unsupported or untested behavior. Closing N1.1 means
this complete inventory is maintained; it does not mean all rows conform.

//! Validation proof contracts. One-shot proofs are exact and unique. Repeated
//! event records have complete typed fields and phase constraints; they never
//! replace their unique aggregate success proof. Live runs also require a fresh
//! serial challenge (see validation_run), so a previous transcript is not a pass.

pub const FULL: &[&str] = &[
    "RECOVERY_BOUNDARY_OK",
    "PAGING_READY",
    "IRQ_READY",
    "IRQ_HARDWARE_ON",
    "USER_ELF_LOADED",
    "USER_ELF_VALIDATED",
    "SCHED_CONTEXT_BENCH_OK",
    "ADDRESS_SPACES_READY",
    "USER_CONTEXT_OK",
    "USER_SYSCALL_OK",
    "USER_FAULT_TERMINATED",
    "USER_OUTPUT_OK",
    "USER_COPY_OK",
    "USER_RECLAIM_OK",
    "USER_CONTEXT_RESUME_OK",
    "USER_PREEMPT_OK",
    "USER_FAULT_ISOLATED",
    "USER_ISOLATION_OK",
    "USERMODE_READY",
    "USER_ELF_LAUNCH_OK",
    "RAMFS_TEMP_CLEAN_OK",
    "RAMFS_TEMP_READY",
    "RAMFS_TEMPORARY_READY",
    "PCI_STORAGE_CONTROLLER_READY",
    "PARTITION_DISCOVERED",
    "BLOCK_CACHE_READY",
    "PERSISTENT_STORAGE_RESTORED",
    "PERSISTENT_STORAGE_READY",
    "BLOCK_CACHE_HIT_OK",
    "VFS_READY",
    "USER_ASYNC_EXIT_OK",
    "USER_OUTPUT_ASYNC_OK",
    "USER_KILL_OK",
    "USER_WAIT_OK",
    "USER_SLEEP_OK",
    "USER_CHILD_WAIT_OK",
    "USER_MESSAGE_OK",
    "USER_COORDINATION_OK",
    "USER_ENDPOINT_CAPABILITY_OK",
    "USER_CHANNEL_FAIRNESS_OK",
    "USER_ENDPOINT_WAKE_OK",
    "USER_FANIN_OK",
    "USER_COPY_OUT_OK",
    "USER_STRUCT_COPY_OK",
    "USER_VFS_BLOCKING_OK",
    "USER_FILE_CAPABILITY_OK",
    "USER_FILE_OFFSET_OK",
    "USER_FILE_CLOSE_OK",
    "USER_ASYNC_ONE_SHOT_OK",
    "USER_ASYNC_REQUEST_ID_OK",
    "USER_ASYNC_CANCELLATION_OK",
    "USER_FILE_WRITE_OK",
    "USER_FILE_WRITE_POLICY_OK",
    "USER_FILE_WRITE_READBACK_OK",
    "USER_INPUT_BLOCK_OK",
    "USER_INPUT_FILTER_OK",
    "USER_INPUT_OWNERSHIP_OK",
    "USER_INPUT_WAKE_OK",
    "USER_ASYNC_LIFECYCLE_OK",
    "USER_PROCESS_LAUNCHED",
    "USER_SUPERVISOR_CLEANUP_OK mode=exit",
    "USER_SUPERVISOR_CLEANUP_OK mode=fault",
    "USER_SUPERVISOR_CLEANUP_OK mode=kill",
    "USER_SUPERVISOR_NO_STALE_TASKS_OK",
    "USER_SUPERVISOR_NO_STALE_HANDLES_OK",
    "USER_SUPERVISOR_PENDING_CANCEL_OK",
    "SUPERVISOR_CLEANUP_READY",
    "USER_PROCESS_KILLED",
    "USER_PROCESS_REAPED",
    "USER_ROLLBACK_FULL_TABLE_OK",
    "USER_ROLLBACK_LAUNCH_REFUSED_OK",
    "USER_ROLLBACK_COPYOUT_OK",
    "USER_ROLLBACK_CANCELLATION_OK",
    "RUNTIME_ROLLBACK_READY",
    "USER_PROCESS_GENERATION_STRESS_OK launches=257",
    "USER_PID_REUSE_SAFE_OK",
    "USER_STALE_PROCESS_HANDLE_REJECTED_OK",
    "PROCESS_GENERATION_STRESS_READY",
    "SCHED_DISPATCH_BENCH_OK",
    "USER_DIRECTORY_READ_OK",
    "USER_STORAGE_STATUS_VISIBLE_OK",
    "USER_RAMFS_TEMP_APP_OK",
    "USER_HANDLE_TRUNCATE_OK",
    "USER_DURABLE_RESTORE_OK",
    "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
    "USER_SOCKET_CAPABILITY_READY abi=18",
    "USER_PROCESS_STATUS",
    "USER_SHELL_PROCESS_CONTROL_OK",
    "USER_SHELL_NAMESPACE_OK",
    "USER_SHELL_HISTORY_OK",
    "USER_SHELL_READY",
    "USER_CONSOLE_TRANSCRIPT_OK commands=2",
    "USER_CONSOLE_HEADLESS_OK",
    "TASKS_READY",
    "SCHED_READY",
    "RUNTIME_COORDINATOR_READY",
    "HEADLESS_RUNTIME_READY",
    "PROCESS_SNAPSHOT_READY",
    "UNIFIED_HANDLE_TABLE_READY",
    "ASYNC_REQUEST_IDENTITY_READY",
    "CONSOLE_TRANSCRIPT_READY",
    "SERVER_TERMINAL_READY",
    "SERIAL_TERMINAL_READY",
    "GENOS_READY",
    "IRQ_TICK_OK",
    "TERMINAL_IDLE_OK",
];

pub const NETWORK: &[&str] = &[
    "NETWORK_DEVICE_READY driver=virtio-net-pci transport=modern-pci",
    "VIRTIO_NET_MSIX_READY vector=48 queues=rx,tx",
    "PACKET_OWNERSHIP_READY",
    "NETWORK_DHCP_READY",
    "ETHERNET_ARP_IPV4_UDP_READY",
    "NETWORK_ICMP_ECHO_OK",
    "IPV6_SLAAC_READY prefix=ra dad=passed",
    "IPV6_ICMP_ECHO_OK",
    "USER_DNS_RESOLVE_OK",
    "USER_HTTP_REQUEST_OK",
    "USER_SOCKET_API_READY",
    "USER_SOCKET_CAPABILITY_READY abi=18",
    "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
    "USER_SOCKET_PASSIVE_LISTEN_READY port=18081",
    "USER_SOCKET_PASSIVE_LISTENER_READY",
    "TCP_PASSIVE_SYN_ACCEPTED",
    "TCP_PASSIVE_HANDSHAKE_OK",
    "USER_SOCKET_PASSIVE_ACCEPT_READY",
    "TCP_PASSIVE_STREAM_RX_OK",
    "TCP_PASSIVE_STREAM_TX_OK",
    "TCP_PASSIVE_STREAM_PEER_FIN_OK",
    "TCP_PASSIVE_STREAM_FIN_OK",
    "USER_SOCKET_PASSIVE_STREAM_READY",
    "USER_SOCKET_PASSIVE_CONCURRENT_READY streams=2",
    "TCP_FAULT_DATA_DROP_INJECTED",
    "TCP_FAULT_REORDER_HELD",
    "TCP_FAULT_REORDER_RELEASED",
    "TCP_STREAM_LARGE_READY bytes=1024 bounded_window=256",
    "TCP_CONGESTION_CONTROL_READY algorithm=aimd max_cwnd=1024",
    "USER_SOCKET_READINESS_WAIT_READY abi=18 wake_budget=2",
    "USER_SOCKET_WAIT_BLOCK",
    "USER_SOCKET_WAIT_WAKE",
    "USER_SOCKET_WAIT_IMMEDIATE",
    "USER_SOCKET_WAIT_TIMEOUT",
    "NETWORK_REGRESSION_BUDGET_OK",
    "USER_SOCKET_TRANSPORT_STARTED protocol=udp",
    "USER_SOCKET_TRANSPORT_COMPLETE protocol=udp",
    "USER_SOCKET_UDP_ASYNC_READY",
    "USER_SOCKET_UDP_TIMEOUT",
    "USER_SOCKET_STALE_REQUEST_DROPPED",
    "USER_SOCKET_TRANSPORT_STARTED protocol=tcp",
    "USER_SOCKET_TRANSPORT_COMPLETE protocol=tcp",
    "USER_SOCKET_TCP_ASYNC_READY",
    "TCP_ASYNC_RESET",
    "USER_SOCKET_STALE_REQUEST_DROPPED protocol=tcp",
    "USER_NETWORK_TIMEOUT_OK",
    "USER_NETWORK_DIAGNOSTICS_READY",
    "USER_SHELL_READY",
    "GENOS_READY",
];

pub const WITHOUT_HTTP: &[&str] = &[
    "NETWORK_DEVICE_READY driver=virtio-net-pci transport=modern-pci",
    "NETWORK_DHCP_READY",
    "IPV6_SLAAC_READY prefix=ra dad=passed",
    "IPV6_ICMP_ECHO_OK",
    "USER_DNS_RESOLVE_OK",
    "USER_SOCKET_CAPABILITY_READY abi=18",
    "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
    "USER_SOCKET_TRANSPORT_STARTED protocol=udp",
    "USER_SOCKET_TRANSPORT_COMPLETE protocol=udp",
    "USER_SOCKET_UDP_ASYNC_READY",
    "USER_SOCKET_UDP_TIMEOUT",
    "USER_SOCKET_STALE_REQUEST_DROPPED",
    "USER_SOCKET_TRANSPORT_STARTED protocol=tcp",
    "TCP_ASYNC_RESET",
    "USER_SOCKET_TCP_ERROR",
    "USER_SOCKET_STALE_REQUEST_DROPPED protocol=tcp",
    "USER_SHELL_READY",
    "GENOS_READY",
];

const REPEATED: &[&str] = &[
    "USER_ELF_LOADED",
    "USER_FAULT_TERMINATED",
    "USER_PROCESS_LAUNCHED",
    "USER_PROCESS_STATUS",
    "USER_PROCESS_KILLED",
    "USER_PROCESS_REAPED",
    "TCP_PASSIVE_SYN_ACCEPTED",
    "TCP_PASSIVE_HANDSHAKE_OK",
    "TCP_PASSIVE_STREAM_RX_OK",
    "TCP_PASSIVE_STREAM_TX_OK",
    "TCP_PASSIVE_STREAM_PEER_FIN_OK",
    "TCP_PASSIVE_STREAM_FIN_OK",
    "TCP_FAULT_DATA_DROP_INJECTED",
    "USER_SOCKET_WAIT_BLOCK",
    "USER_SOCKET_WAIT_WAKE",
    "USER_SOCKET_WAIT_IMMEDIATE",
    "USER_SOCKET_WAIT_TIMEOUT",
    "USER_SOCKET_TRANSPORT_STARTED",
    "TCP_ASYNC_RESET",
    "USER_SOCKET_TCP_ERROR",
];

fn template(marker: &str) -> String {
    let key = marker.split(' ').next().unwrap_or(marker);
    let suffix = match key {
        "FRAME_ALLOCATOR_BITMAP_READY" => "capacity_gib=8",
        "PAGING_READY" => "root={hex} tables={positive}",
        "ADDRESS_SPACES_READY" => "count=3",
        "USER_ELF_VALIDATED" => "entry={hex} segments={positive} pages={positive} bytes={positive}",
        "USER_ELF_LOADED" => "pid={pid} root={hex}",
        "USER_ELF_LAUNCH_OK" => "pid={pid} preemptions={positive}",
        "USER_FAULT_TERMINATED" => "pid={pid} vector={u64} error={hex} rip={hex} cr2={hex}",
        "RAMFS_TEMP_READY" => "path=/TMP/SESSION.TXT",
        "PCI_STORAGE_CONTROLLER_READY" => "vendor=0x8086 device=0x7010 prog_if=0x80 io=0x1f0 control=0x3f6",
        "PARTITION_DISCOVERED" if marker.contains("type=0x7e") => "scheme=mbr type=0x7e start=64 sectors=16320",
        "PARTITION_DISCOVERED" => "scheme=mbr type=0x7f start=64 sectors=16320",
        "BLOCK_CACHE_READY" => "entries=8 policy=write-back",
        "PERSISTENT_STORAGE_RESTORED" => "generation={positive}",
        "USER_DURABLE_RESTORE_OK" | "USER_DURABLE_WRITE_OK" => "path=/USER/SHELL.TXT",
        "USER_HANDLE_TRUNCATE_OK" | "USER_SOCKET_WAIT_BLOCK" | "USER_SOCKET_WAIT_WAKE"
        | "USER_SOCKET_WAIT_TIMEOUT" | "USER_SOCKET_UDP_TIMEOUT" | "USER_SOCKET_TCP_ERROR" => "pid={pid}",
        "USER_PROCESS_LAUNCHED" => "owner={pid} pid={pid} handle={hex} mode=hold",
        "USER_PROCESS_STATUS" => "owner={pid} target={pid}",
        "USER_PROCESS_KILLED" => "owner={pid} pid={pid} code=137",
        "USER_PROCESS_REAPED" => "owner={pid} pid={pid}",
        "SERVER_TERMINAL_READY" => "mode=serial ui=off",
        "SERIAL_TERMINAL_READY" => "port=com1",
        "PACKET_OWNERSHIP_READY" => "buffers=8 states=free,driver,stack",
        "NETWORK_DHCP_READY" => "address=10.0.2.15 gateway=10.0.2.2 dns=10.0.2.3",
        "USER_SOCKET_STALE_REQUEST_DROPPED" if !marker.contains("protocol=") => "protocol=udp",
        "NETWORK_REGRESSION_BUDGET_OK" => "bytes={positive} elapsed_ticks={positive} throughput_milli_bytes_per_tick={positive} max_ack_ticks={u64} retransmissions={positive} reordered={positive} max_buffered={u64} max_stream_bytes={positive} congestion_events={positive} cwnd_min=128 cwnd_max={positive} irq_completions={positive} recovery_completions={u64} recovery_polls={u64} frames={positive} notifications={u64} queue_capacity=8 observed_tick={positive}",
        _ => return marker.to_string(),
    };
    format!("{key} {suffix}")
}

fn field_matches(pattern: &str, value: &str) -> bool {
    match pattern {
        "{hex}" => value.strip_prefix("0x").is_some_and(|digits| {
            !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_hexdigit())
                && u64::from_str_radix(digits, 16).is_ok()
        }),
        "{u64}" | "{positive}" | "{pid}" => {
            !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_digit())
                && value.parse::<u64>().is_ok_and(|n| match pattern {
                    "{positive}" => n > 0,
                    "{pid}" => (1..=255).contains(&n),
                    _ => true,
                })
        }
        _ => pattern == value,
    }
}

fn matches(pattern: &str, line: &str) -> bool {
    let expected: Vec<_> = pattern.split(' ').collect();
    let fields: Vec<_> = line.split(' ').collect();
    expected.len() == fields.len()
        && expected.iter().zip(fields).all(|(want, got)| {
            match (want.split_once('='), got.split_once('=')) {
                (Some((key, pattern)), Some((actual, value))) => {
                    key == actual && field_matches(pattern, value)
                }
                _ => *want == got,
            }
        })
}

fn normalized(line: &str) -> &str {
    // The serial terminal writes its prompt without a newline. These two
    // asynchronous records may consume it; no arbitrary prefix is accepted.
    match line {
        "genos> IRQ_TICK_OK" => "IRQ_TICK_OK",
        "genos> TERMINAL_IDLE_OK" => "TERMINAL_IDLE_OK",
        _ => line,
    }
}

fn key(marker: &str) -> &str {
    marker.split(' ').next().unwrap_or(marker)
}

#[derive(Clone, Copy)]
pub struct Contract<'a> {
    pub required: &'a [&'a str],
    pub full: bool,
    pub network: bool,
    pub without_http: bool,
}

impl<'a> Contract<'a> {
    pub fn phase(required: &'a [&'a str], full: bool) -> Self {
        Self {
            required,
            full,
            network: false,
            without_http: false,
        }
    }
    pub fn network(without_http: bool) -> Self {
        Self {
            required: if without_http { WITHOUT_HTTP } else { NETWORK },
            full: false,
            network: true,
            without_http,
        }
    }

    /// A prefix may be incomplete, but malformed, duplicated and wrong-phase
    /// evidence is rejected as soon as its contradiction is observable.
    pub fn assess(&self, output: &str) -> Result<bool, String> {
        let lines: Vec<_> = output
            .lines()
            .map(|line| normalized(line.trim_end_matches('\r')))
            .collect();
        for line in &lines {
            if [
                "KERNEL PANIC",
                "BOOTLOADER_PANIC",
                "_FAILED",
                "EXCEPTION_FATAL_HALT",
                "RECOVERY_CONSOLE_READY",
                "userspace lifecycle error",
                "terminal input delivery failed",
                "BOOT_MODE normal",
            ]
            .iter()
            .any(|bad| line.contains(bad))
            {
                return Err(format!("validation failure: {line}"));
            }
            if self.without_http
                && [
                    "USER_HTTP_REQUEST_OK",
                    "USER_SOCKET_TCP_ASYNC_READY",
                    "USER_SOCKET_PASSIVE_ACCEPT_READY",
                    "USER_SOCKET_PASSIVE_STREAM_READY",
                    "USER_SOCKET_PASSIVE_CONCURRENT_READY",
                ]
                .iter()
                .any(|bad| key(line) == *bad)
            {
                return Err(format!(
                    "test-only HTTP/passive service unexpectedly succeeded: {line}"
                ));
            }
        }
        let mut required = vec!["BOOT_MODE validation"];
        required.extend(crate::cpu_evidence::BOOT_SEQUENCE.iter().copied());
        required.extend([
            "USERMODE_READY",
            "USER_SHELL_READY",
            "CONSOLE_TRANSCRIPT_READY",
            "GENOS_READY",
        ]);
        if self.full {
            required.extend(FULL.iter().copied());
        }
        required.extend(self.required.iter().copied());
        required.sort_unstable();
        required.dedup();
        let patterns: Vec<_> = required.iter().map(|m| template(m)).collect();
        let mut positions = vec![Vec::new(); required.len()];
        let mut complete = true;
        for (position, line) in lines.iter().enumerate() {
            let mut recognized = false;
            let mut valid = false;
            for (i, marker) in required.iter().enumerate() {
                if key(line) == key(marker) {
                    recognized = true;
                    if matches(&patterns[i], line) {
                        if key(marker) == "NETWORK_REGRESSION_BUDGET_OK"
                            && !network_budget_valid(line)
                        {
                            return Err(
                                "network regression counters violate the frozen bounds".into()
                            );
                        }
                        positions[i].push(position);
                        valid = true;
                    }
                }
            }
            if recognized && !valid {
                return Err(format!("malformed proof fields: {line}"));
            }
            // Do not accept a success token embedded in a different log record.
            if !recognized && required.iter().any(|m| line.contains(key(m))) {
                return Err(format!("embedded proof record: {line}"));
            }
        }
        let locate = |marker: &str| {
            required
                .iter()
                .position(|m| *m == marker)
                .and_then(|i| positions[i].first().copied())
        };
        let policy = locate("BOOT_MODE validation");
        let ready = locate("GENOS_READY");
        for (i, marker) in required.iter().enumerate() {
            let found = &positions[i];
            if found.is_empty() {
                complete = false;
                continue;
            }
            if !REPEATED.contains(&key(marker)) && found.len() != 1 {
                return Err(format!("duplicate one-shot proof: {marker}"));
            }
            if *marker == "BOOT_MODE validation" || *marker == "GENOS_READY" {
                continue;
            }
            let terminal = matches!(*marker, "IRQ_TICK_OK" | "TERMINAL_IDLE_OK");
            for position in found {
                if !policy.is_some_and(|start| *position > start)
                    || (terminal && !ready.is_some_and(|start| *position > start))
                    || (!terminal && ready.is_some_and(|end| *position >= end))
                {
                    return Err(format!("proof outside its boot phase: {marker}"));
                }
            }
        }
        // Admission order is common to every validation image, including the
        // storage, SDK, serial and optional-server variants.
        let boot: Vec<_> = std::iter::once("BOOT_MODE validation")
            .chain(crate::cpu_evidence::BOOT_SEQUENCE.iter().copied())
            .chain([
                "USERMODE_READY",
                "USER_SHELL_READY",
                "CONSOLE_TRANSCRIPT_READY",
                "GENOS_READY",
            ])
            .collect();
        self.ordered(&boot, &required, &positions)?;
        if self.full {
            // One-shot full-smoke proofs are emitted by sequential production
            // stages. Repeatable events retain their own typed/phase contract.
            let ordered: Vec<_> = FULL
                .iter()
                .copied()
                .filter(|m| !REPEATED.contains(&key(m)))
                .collect();
            self.ordered(&ordered, &required, &positions)?;
            self.ordered(
                &[
                    "USER_SOCKET_CAPABILITY_READY abi=18",
                    "USER_PROCESS_LAUNCHED",
                    "USER_PROCESS_STATUS",
                    "USER_PROCESS_KILLED",
                    "USER_PROCESS_REAPED",
                    "USER_SHELL_PROCESS_CONTROL_OK",
                ],
                &required,
                &positions,
            )?;
        }
        self.ordered(&["MEMORY_HYGIENE_READY bytes=4096 invalid_free=denied reused=zero",
            "FRAME_OWNERSHIP_READY stale=denied foreign=denied alias=denied pinned=denied reclaimed=true",
            "MEMORY_PRESSURE_READY owner_limit=64 isolated=true reclaimed=true",
            "USER_COPY_READY bounded=true permissions=true atomic=true stale=denied",
            "TLB_RETIREMENT_READY switched=true reused=true reclaimed=true",
            "IRQ_CRITICAL_SECTION_READY nested=preserved outer=restored"], &required, &positions)?;
        if self.network {
            for chain in [
                &[
                    "VIRTIO_NET_MSIX_READY vector=48 queues=rx,tx",
                    "NETWORK_DEVICE_READY driver=virtio-net-pci transport=modern-pci",
                    "NETWORK_DHCP_READY",
                    "GENOS_READY",
                ][..],
                &[
                    "USER_SOCKET_TRANSPORT_STARTED protocol=udp",
                    "USER_SOCKET_TRANSPORT_COMPLETE protocol=udp",
                    "USER_SOCKET_UDP_ASYNC_READY",
                    "USER_SOCKET_UDP_TIMEOUT",
                    "USER_SOCKET_STALE_REQUEST_DROPPED",
                    "USER_SHELL_READY",
                ],
                &[
                    "USER_SOCKET_TRANSPORT_STARTED protocol=tcp",
                    "USER_SOCKET_STALE_REQUEST_DROPPED protocol=tcp",
                    "USER_SHELL_READY",
                ],
                &[
                    "USER_SOCKET_PASSIVE_LISTENER_READY",
                    "TCP_PASSIVE_SYN_ACCEPTED",
                    "TCP_PASSIVE_HANDSHAKE_OK",
                    "USER_SOCKET_PASSIVE_ACCEPT_READY",
                    "USER_SOCKET_PASSIVE_STREAM_READY",
                    "USER_SOCKET_PASSIVE_CONCURRENT_READY streams=2",
                    "USER_SHELL_READY",
                ],
                &[
                    "TCP_FAULT_REORDER_HELD",
                    "TCP_FAULT_REORDER_RELEASED",
                    "NETWORK_REGRESSION_BUDGET_OK",
                    "TCP_STREAM_LARGE_READY bytes=1024 bounded_window=256",
                    "TCP_CONGESTION_CONTROL_READY algorithm=aimd max_cwnd=1024",
                ],
                &[
                    "USER_HTTP_REQUEST_OK",
                    "USER_SOCKET_API_READY",
                    "USER_NETWORK_TIMEOUT_OK",
                    "USER_NETWORK_DIAGNOSTICS_READY",
                    "USER_SHELL_READY",
                ],
            ] {
                self.ordered(chain, &required, &positions)?;
            }
        }
        Ok(complete
            && stack_usage_ready(&lines)?
            && crate::cpu_evidence::validation_ready(&lines.join("\n")))
    }

    fn ordered(
        &self,
        chain: &[&str],
        required: &[&str],
        positions: &[Vec<usize>],
    ) -> Result<(), String> {
        let mut previous = None;
        for marker in chain {
            let Some(index) = required.iter().position(|m| m == marker) else {
                continue;
            };
            if positions[index].is_empty() {
                continue;
            }
            let position = positions[index]
                .iter()
                .copied()
                .find(|p| previous.is_none_or(|earlier| *p > earlier));
            let Some(position) = position else {
                return Err(format!("out-of-order proof: {marker}"));
            };
            previous = Some(position);
        }
        Ok(())
    }
}

fn network_budget_valid(line: &str) -> bool {
    let values: Vec<u64> = line
        .split(' ')
        .skip(1)
        .filter_map(|field| field.split_once('=')?.1.parse().ok())
        .collect();
    if values.len() != 18 {
        return false;
    }
    let [bytes, elapsed, throughput, ack, retransmissions, reordered, buffered, stream_bytes, congestion, cwnd_min, cwnd_max, irq, recovery, polls, frames, notifications, capacity, tick] =
        values[..]
    else {
        return false;
    };
    bytes >= 1024
        && (1..=800).contains(&elapsed)
        && tick >= elapsed
        && throughput > 0
        && throughput == bytes.saturating_mul(1000) / elapsed
        && ack <= 100
        && (1..=12).contains(&retransmissions)
        && reordered > 0
        && buffered <= 256
        && stream_bytes >= 1024
        && congestion >= 3
        && cwnd_min == 128
        && (129..=1024).contains(&cwnd_max)
        && frames > 0
        && recovery <= frames / 16 + 1
        && irq > recovery.saturating_mul(8)
        && notifications <= frames.saturating_mul(3)
        && polls <= frames.saturating_mul(128)
        && capacity == 8
}

fn stack_usage_ready(lines: &[&str]) -> Result<bool, String> {
    let names = [
        ("boot", 2097152),
        ("irq", 65536),
        ("privilege", 65536),
        ("double-fault", 16384),
        ("nmi", 16384),
        ("machine-check", 16384),
        ("debug", 16384),
    ];
    let after = lines
        .iter()
        .position(|line| *line == "CONSOLE_TRANSCRIPT_READY");
    let before = lines.iter().position(|line| *line == "GENOS_READY");
    let mut seen = [false; 7];
    for (position, line) in lines.iter().enumerate() {
        if !line.contains("KERNEL_STACK_USAGE") {
            continue;
        }
        if line.split(' ').count() != 5 || !line.starts_with("KERNEL_STACK_USAGE ") {
            return Err("malformed stack-usage record".into());
        }
        let mut fields = line.split(' ').skip(1);
        let name = fields
            .next()
            .and_then(|f| f.strip_prefix("name="))
            .ok_or("missing stack name")?;
        let index = names
            .iter()
            .position(|(expected, _)| *expected == name)
            .ok_or("unknown stack name")?;
        let numbers: Result<Vec<u64>, String> = ["capacity=", "touched=", "remaining="]
            .into_iter()
            .map(|prefix| {
                let value = fields
                    .next()
                    .and_then(|f| f.strip_prefix(prefix))
                    .ok_or("malformed stack counter")?;
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("malformed stack counter".into());
                }
                value
                    .parse()
                    .map_err(|_| "overflowing stack counter".into())
            })
            .collect();
        let values = numbers?;
        if seen[index]
            || fields.next().is_some()
            || values[0] != names[index].1
            || values[1].checked_add(values[2]) != Some(values[0])
            || values.iter().any(|n| n % 8 != 0)
            || values[2] < 4096
            || (index < 3 && values[1] == 0)
            || !after.is_some_and(|a| position > a)
            || before.is_some_and(|b| position >= b)
        {
            return Err(format!(
                "invalid, duplicate or wrong-phase stack usage: {name}"
            ));
        }
        seen[index] = true;
    }
    Ok(seen.into_iter().all(|present| present))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALIDATION: &str = include_str!("../tests/fixtures/validation.log");
    const NETWORK_LOG: &str = include_str!("../tests/fixtures/network.log");
    const WITHOUT_HTTP_LOG: &str = include_str!("../tests/fixtures/without-http.log");

    #[test]
    fn captured_record_shapes_complete_each_contract() {
        for (contract, log) in [
            (Contract::phase(&[], true), VALIDATION),
            (Contract::network(false), NETWORK_LOG),
            (Contract::network(true), WITHOUT_HTTP_LOG),
        ] {
            assert_eq!(contract.assess(log), Ok(true));
        }
    }

    #[test]
    fn every_required_proof_rejects_omission_embedding_and_wrong_phase() {
        for (contract, log) in [
            (Contract::phase(&[], true), VALIDATION),
            (Contract::network(false), NETWORK_LOG),
            (Contract::network(true), WITHOUT_HTTP_LOG),
        ] {
            let required = if contract.full {
                FULL
            } else {
                contract.required
            };
            for marker in required {
                let pattern = template(marker);
                let matching: Vec<_> = log
                    .lines()
                    .filter(|l| matches(&pattern, normalized(l)))
                    .collect();
                assert!(!matching.is_empty(), "missing test fixture: {marker}");
                let omitted = log
                    .lines()
                    .filter(|l| !matches(&pattern, normalized(l)))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_ne!(contract.assess(&omitted), Ok(true), "omitted {marker}");
                let forged = format!("{omitted}\nforged {}\n", matching[0]);
                assert!(contract.assess(&forged).is_err(), "embedded {marker}");
                if !REPEATED.contains(&key(marker)) {
                    assert!(
                        contract.assess(&format!("{log}{}\n", matching[0])).is_err(),
                        "duplicate {marker}"
                    );
                }
                if *marker != "GENOS_READY" {
                    let wrong_phase = format!("{}\n{omitted}\n", normalized(matching[0]));
                    assert!(
                        contract.assess(&wrong_phase).is_err(),
                        "wrong phase {marker}"
                    );
                }
                let forged_fields =
                    format!("{omitted}\n{} injected=true\n", normalized(matching[0]));
                assert!(
                    contract.assess(&forged_fields).is_err(),
                    "extra fields {marker}"
                );
            }
        }
    }

    #[test]
    fn repeating_a_diagnostic_cannot_replace_unique_success_evidence() {
        let contract = Contract::network(false);
        let log = NETWORK_LOG.replace(
            "USER_SOCKET_PASSIVE_CONCURRENT_READY streams=2\n",
            "TCP_PASSIVE_HANDSHAKE_OK\n",
        );
        assert_ne!(contract.assess(&log), Ok(true));
        assert!(contract
            .assess(&NETWORK_LOG.replace("retransmissions=3", "retransmissions=garbage"))
            .is_err());
        assert!(contract
            .assess(&NETWORK_LOG.replace("pid=33", "pid=0"))
            .is_err());
    }

    #[test]
    fn memory_authority_proofs_require_unique_ordered_exact_results() {
        let markers = [
            "MEMORY_HYGIENE_READY bytes=4096 invalid_free=denied reused=zero",
            "FRAME_OWNERSHIP_READY stale=denied foreign=denied alias=denied pinned=denied reclaimed=true",
            "MEMORY_PRESSURE_READY owner_limit=64 isolated=true reclaimed=true",
            "USER_COPY_READY bounded=true permissions=true atomic=true stale=denied",
            "TLB_RETIREMENT_READY switched=true reused=true reclaimed=true",
            "IRQ_CRITICAL_SECTION_READY nested=preserved outer=restored",
        ];
        let log = VALIDATION.replace(
            "USERMODE_READY\n",
            &format!("{}\nUSERMODE_READY\n", markers.join("\n")),
        );
        let contract = Contract::phase(&markers, false);
        assert_eq!(contract.assess(&log), Ok(true));
        for (index, marker) in markers.iter().enumerate() {
            assert_ne!(
                contract.assess(&log.replace(&format!("{marker}\n"), "")),
                Ok(true)
            );
            assert!(contract
                .assess(&log.replace(marker, &format!("{marker}\n{marker}")))
                .is_err());
            assert!(contract
                .assess(&log.replace(marker, &format!("forged {marker}")))
                .is_err());
            let mut swapped = markers;
            swapped.swap(index, (index + 1) % markers.len());
            assert!(contract
                .assess(&log.replace(&markers.join("\n"), &swapped.join("\n")))
                .is_err());
        }
    }

    #[test]
    fn numeric_success_labels_do_not_override_network_or_stack_budgets() {
        for (before, after) in [
            ("retransmissions=3", "retransmissions=13"),
            ("max_buffered=256", "max_buffered=257"),
            ("congestion_events=3", "congestion_events=2"),
            ("cwnd_max=1024", "cwnd_max=1025"),
            (
                "throughput_milli_bytes_per_tick=10141",
                "throughput_milli_bytes_per_tick=1",
            ),
        ] {
            assert!(
                Contract::network(false)
                    .assess(&NETWORK_LOG.replace(before, after))
                    .is_err(),
                "{after}"
            );
        }
        for line in VALIDATION
            .lines()
            .filter(|l| l.starts_with("KERNEL_STACK_USAGE "))
        {
            assert_ne!(
                Contract::phase(&[], true).assess(&VALIDATION.replace(&format!("{line}\n"), "")),
                Ok(true)
            );
            assert!(Contract::phase(&[], true)
                .assess(&format!("{VALIDATION}{line}\n"))
                .is_err());
            assert!(Contract::phase(&[], true)
                .assess(&VALIDATION.replace(line, &format!("forged {line}")))
                .is_err());
        }
        for bad in [
            "KERNEL_STACK_USAGE name=boot capacity=2097152 touched=0 remaining=2097152",
            "KERNEL_STACK_USAGE name=irq capacity=65536 touched=65528 remaining=8",
            "KERNEL_STACK_USAGE name=privilege capacity=65536 touched=1023 remaining=64513",
        ] {
            let name = bad.split(' ').nth(1).unwrap();
            let original = VALIDATION
                .lines()
                .find(|l| l.starts_with(&format!("KERNEL_STACK_USAGE {name} ")))
                .unwrap();
            assert!(Contract::phase(&[], true)
                .assess(&VALIDATION.replace(original, bad))
                .is_err());
        }
    }

    #[test]
    fn unsupported_http_success_and_crashes_cannot_pass() {
        for bad in [
            "USER_HTTP_REQUEST_OK",
            "USER_SOCKET_TCP_ASYNC_READY",
            "KERNEL PANIC",
            "RECOVERY_CONSOLE_READY",
            "BOOT_MODE normal",
            "USER_ELF_LOAD_FAILED",
        ] {
            assert!(Contract::network(true)
                .assess(&format!("{WITHOUT_HTTP_LOG}{bad}\n"))
                .is_err());
        }
    }
}

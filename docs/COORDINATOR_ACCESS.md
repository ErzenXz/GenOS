# Network and storage coordinator access

`network::NETWORK` owns the protocol stack and device object in `ScopedCell`.
`storage::STORAGE` owns the persistent-slot scratch buffer, discovered ATA port
pair and mount phase together. Public network observations and mutations use the
same private callback adapter; mount, commit and rollback callbacks hold the
storage scope. No state reference escapes a callback. ATA helpers receive the
owned port pair explicitly rather than reading mutable global port numbers.

`ScopedCell` owns its `UnsafeCell<T>` permanently. A successful acquire CAS
establishes exclusive access before its single pointer dereference. Failed
contenders return `Busy` without dereferencing the value or executing callbacks.
A private RAII guard releases the flag after the mutable borrow ends, including
host unwinding. Release/acquire orders handoff of prior value writes. Private
fields and the higher-ranked callback prevent safe code from retaining a value
borrow after flag release. Owned results may leave the scope. No spinning,
poisoning, rollback or interrupt-state changes occur in this primitive.

The new `unsafe impl<T: Send> Sync` allows exclusive transfer between threads;
`T: Sync` is unnecessary because concurrent shared value references are never
exposed. `T: Send` is mandatory because the exclusive access may cross threads.
A compile-fail check rejects `ScopedCell<Rc<_>>` as Sync; another rejects a
returned value borrow. This review covers this cell's two unsafe constructs,
not all repository boundaries. Rust's [UnsafeCell contract](https://doc.rust-lang.org/core/cell/struct.UnsafeCell.html)
retains mutable-reference uniqueness and data-race requirements.

Hardware adapters still require exactly one active BSP and synchronous
coordinator ownership. Timer handlers do not switch kernel threads. Ordinary
network IRQ handlers publish atomic readiness and acknowledge interrupts;
ordinary IRQ handlers never borrow either coordinator object. Emergency entries
remain fatal assembly paths and must not access these objects or their reporters
through an interrupted Rust context. The cell does not establish DMA/ring
ownership, device reset retirement, SMP support, latency bounds or interrupt
nesting qualification. Those requirements remain open in F4/F5 and driver work.

Callbacks preserve the caller's IF state. Network completion requires IRQs to
remain available; storage uses bounded polling without extending a global IRQ
mask. A nested borrow fails before access, then the adapter disables IRQs,
prints `NETWORK_ACCESS_REJECTED policy=halt` or
`STORAGE_ACCESS_REJECTED policy=halt`, and halts. It never waits for the interrupted
borrow holder. This is fail-stop containment, not a recoverable lock API.

Network initialization and storage mount may each begin once per boot. The phase
flag is set before discovery, so an unavailable device or failed initialization
also consumes that attempt. Repeating the operation requires the distinct
`NETWORK_INIT_REJECTED policy=halt` or `STORAGE_MOUNT_REJECTED policy=halt` refusal
before rediscovery or I/O. Recovery from that boot-level attempt requires a fresh
boot. Existing internal network driver recovery and storage commit/quarantine
semantics are unchanged; a storage device error still restores the RAM snapshot
and enters the existing read-only policy.

Seven cell tests run both on the host and under the pinned Miri lane, covering
reentry, mutation, unwind release, sequential/nested scopes, a held writer with
contenders, and Send-but-not-Sync handoff. `test_coordinator_access.py` archives
clean source and independently builds four software-only VM fixtures for nested
network/storage borrows and repeated initialization/mount. They preserve the
production cell, adapter and fatal reporter, require exact serial ordering and
no continuation, and retain raw paused-start QMP status/serial and hashed source,
fixture, image, environment and result manifests. Discovery may report unavailable;
no network or dedicated data disk is attached to these refusal-only fixtures.
They do not execute CPU fault injection or qualify actual device concurrency.
The production fresh-volume write-failure fixture and full system suite provide
separate storage recovery and working network/driver regression evidence.

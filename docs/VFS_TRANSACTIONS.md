# Scoped persistent VFS mutations

The single BSP runtime's permanent rollback buffer is an owned `VfsSnapshot`
inside `ScopedCell`. `RuntimeCoordinator::mutate_vfs` holds its one borrow from
capture through mutation, storage commit, restore and failure-status publication.
Writable open/create, offset write, truncate, directory create and path removal
all use this adapter. There are no separate raw capture/restore functions or
unscoped mutable rollback accesses.

`RamVfs::copy_from` copies its owned node array and length through safe Rust.
`VfsSnapshot::with_snapshot` captures the complete live view before its callback
receives separate mutable live and immutable saved views. Exclusive snapshot
access prevents replacement during that callback; higher-ranked lifetimes prevent
references escaping in the return value. The permanent buffer avoids another
full VFS field in each runtime coordinator. No heap allocation, file/path/node
limit, ABI or media format changes are involved.

A failed RAM mutation restores the saved view and skips storage. On a successful
RAM mutation, production `PersistentFs::sync` receives a restore callback borrowing
that exact snapshot. For rejected or failed commits, storage restores first and
then publishes quarantine status. The runtime does not restore again afterward:
that would erase the newly published read-only/unknown-outcome diagnostic. Only
a committed result acknowledges the mutation to the process. Existing uncertain
media outcomes and fresh-boot recovery policy remain unchanged.

The scope preserves caller IF and does not wait. Reentry is refused before value
access, disables IRQs, prints `VFS_TRANSACTION_REJECTED policy=halt`, then halts.
The acquisition order is rollback snapshot then storage; the restore callback
uses its already-borrowed snapshot and never reacquires either scope.
Ordinary IRQ handlers never enter runtime VFS mutation or storage; no kernel
thread switch occurs within the callback. Emergency entries remain fatal and
must not access this Rust state. This does not establish SMP, device retirement,
lock-order enforcement, arbitrary interrupt nesting or worst-case latency.
Host panic unwinding releases the cell borrow but is not a transaction rollback
contract; a kernel panic is fatal.

Five pure tests cover capture ordering, complete full-capacity/file-byte restore,
latest acknowledged view reuse, independent snapshot owners and failure-status
publication after restore. A compile-fail check rejects an escaping snapshot
borrow. These tests join the pinned Miri lane. The production storage error matrix
uses the owned snapshot through every write/flush cutoff, before/after error and
write-through mode (176 cases), retaining RAM/media/quarantine assertions and
checking the exact failure status. The `vfs-borrow` VM fixture holds a real
snapshot callback while reentering the production rollback adapter; it requires
the distinct refusal without continuation, fault, reset or VM exit. Full system
storage and process/file workflows supply separate integration evidence.

Two raw-copy unsafe blocks were removed; this change adds no unsafe construct or
Send/Sync implementation. The broader F4/F5 ownership and unsafe review remains
open; see [coordinator access](COORDINATOR_ACCESS.md) for the reused cell's unsafe
contract and hardware restrictions.

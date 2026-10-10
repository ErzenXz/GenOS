use genos_abi::{BootInfo, MemoryRegionKind};
pub use kernel::frame_grant::{Grant, Kind, Owner};
use kernel::frame_grant::{Ledger, GRANT_CAPACITY};
use kernel::physmem::{scrub_page, FrameAllocator, KERNEL_BITMAP_WORDS, PAGE_SIZE};

// Permanent in-place storage avoids copying the bitmap/ledger onto a stack.
// The admitted BSP owns it; only with_state creates references to it.
struct MemoryState {
    allocator: FrameAllocator<KERNEL_BITMAP_WORDS>,
    grants: Ledger<GRANT_CAPACITY>,
    boot_started: bool,
    ready: bool,
    #[cfg(feature = "memory-test-faults")]
    fail_after: Option<usize>,
}

static mut STATE: MemoryState = MemoryState {
    allocator: FrameAllocator::new(),
    grants: Ledger::new(),
    boot_started: false,
    ready: false,
    #[cfg(feature = "memory-test-faults")]
    fail_after: None,
};
static ACCESS: kernel::context_slot::ContextSlot<MemoryState> =
    kernel::context_slot::ContextSlot::new();

fn reject(message: &str) -> ! {
    crate::arch::disable_interrupts();
    crate::serial::println(message);
    crate::arch::halt_loop();
}

// Private, non-escaping callbacks cannot enable interrupts, switch contexts or
// reenter memory. Both slot guards retire before prior IF is restored.
fn with_state<R>(operation: impl for<'a> FnOnce(&'a mut MemoryState) -> R) -> R {
    crate::arch::without_interrupts(|| {
        let pointer = core::ptr::NonNull::new(core::ptr::addr_of_mut!(STATE)).unwrap();
        // SAFETY: BSP admission excludes other CPUs; local IRQs are masked.
        // STATE is permanent/stationary, with no owner reference or other access.
        // Emergency/NMI entries cannot enter memory Rust code. The slot refuses
        // reentry before dereference, and the HRTB prevents a state borrow escaping.
        let result = unsafe { ACCESS.run(pointer, || ACCESS.with_current(operation)) };
        match result {
            Ok(Ok(Some(value))) => value,
            _ => reject("MEMORY_ACCESS_REJECTED policy=halt"),
        }
    })
}

fn with_ready_state<R>(operation: impl for<'a> FnOnce(&'a mut MemoryState) -> R) -> R {
    with_state(|state| {
        if !state.ready {
            reject("MEMORY_PHASE_REJECTED reason=uninitialized policy=halt");
        }
        operation(state)
    })
}

pub fn init(boot_info: &BootInfo) {
    if crate::arch::interrupts_enabled() {
        reject("MEMORY_CONTEXT_REJECTED reason=initialization-if policy=halt");
    }
    let (total, regions) = with_state(|state| {
        if state.boot_started {
            reject("MEMORY_PHASE_REJECTED reason=already-initialized policy=halt");
        }
        state.boot_started = true;
        let Ok(regions) = genos_abi::boot_memory::validate_map(&boot_info.memory_map) else {
            reject("FRAME_ALLOCATOR_MAP_REJECTED invalid_handoff=true");
        };
        for region in regions {
            if region.kind == MemoryRegionKind::Usable && !state.allocator.add_region(*region) {
                reject("FRAME_ALLOCATOR_MAP_REJECTED overlap_or_capacity=true");
            }
        }
        assert!(state.allocator.is_consistent());
        state.ready = true;
        // Return copies; serial output does not retain the allocator borrow.
        (
            state.allocator.usable_bytes(),
            state.allocator.region_count(),
        )
    });
    crate::serial::print("Usable memory bytes: ");
    crate::serial::print_u64(total);
    crate::serial::print(" regions=");
    crate::serial::print_u64(regions as u64);
    crate::serial::println("");
    crate::serial::println("FRAME_ALLOCATOR_BITMAP_READY capacity_gib=8");
}

/// All allocator/ledger access shares one IRQ-masked, reentry-checked scope.
/// Emergency paths cannot call it; local IRQ masking is not an SMP lock.
fn with_allocator<R>(
    operation: impl for<'a> FnOnce(&'a mut FrameAllocator<KERNEL_BITMAP_WORDS>) -> R,
) -> R {
    with_memory(|allocator, _| operation(allocator))
}

fn with_memory<R>(
    operation: impl for<'a> FnOnce(
        &'a mut FrameAllocator<KERNEL_BITMAP_WORDS>,
        &'a mut Ledger<GRANT_CAPACITY>,
    ) -> R,
) -> R {
    with_ready_state(|state| operation(&mut state.allocator, &mut state.grants))
}

#[allow(dead_code)]
pub fn usable_bytes() -> u64 {
    with_allocator(|allocator| allocator.usable_bytes())
}

/// Return an exclusively granted, zero-filled identity-mapped physical frame.
pub fn alloc_frame(owner: Owner, kind: Kind) -> Option<Grant> {
    with_ready_state(|state| {
        #[cfg(feature = "memory-test-faults")]
        {
            if let Some(remaining) = state.fail_after {
                if remaining == 0 {
                    return None;
                }
                state.fail_after = Some(remaining - 1);
            }
        }
        let grant = state
            .grants
            .allocate(owner, kind, || state.allocator.alloc_frame())?;
        let frame = grant.address();
        // SAFETY: the bitmap just granted this usable, aligned page exclusively.
        // Firmware/retained supervisor identity mappings cover managed RAM.
        // It is zeroed before any caller can publish a user or table mapping.
        unsafe {
            core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
        }
        Some(grant)
    })
}

#[cfg(feature = "memory-test-faults")]
pub fn fail_after(allocations: Option<usize>) {
    with_ready_state(|state| state.fail_after = allocations);
}

/// Validate allocation identity and mapping pins before touching the page.
///
/// # Safety
/// The grant holder must retire Rust references, hardware translations and any
/// device access before release. Possession of an address is not authority.
pub unsafe fn free_frame(grant: Grant) -> bool {
    with_memory(|allocator, grants| {
        grants.release(grant, |frame| {
            allocator.free_frame_with(frame, |frame| {
                // SAFETY: ledger identity and the live bitmap were validated before
                // entering this callback. Caller retired users; the BSP owns this
                // page with IRQs masked and a retained supervisor identity mapping.
                let page = unsafe { &mut *(frame as *mut [u8; PAGE_SIZE as usize]) };
                scrub_page(page);
                true
            })
        })
    })
}

pub fn new_owner() -> Option<Owner> {
    with_memory(|_, grants| grants.new_owner())
}
pub fn is_live(grant: Grant) -> bool {
    with_memory(|_, grants| grants.is_live(grant))
}
pub fn find_grant(owner: Owner, frame: u64, kind: Kind) -> Option<Grant> {
    with_memory(|_, grants| grants.find(owner, frame, kind))
}
pub fn can_map(grant: Grant, owner: Owner) -> bool {
    with_memory(|_, grants| grants.can_bind(grant, owner))
}
pub fn pin_mapping(grant: Grant, owner: Owner, address: u64) -> bool {
    with_memory(|_, grants| grants.bind(grant, owner, address))
}
pub fn mapping_matches(owner: Owner, frame: u64, address: u64) -> bool {
    with_memory(|_, grants| grants.mapping_matches(owner, frame, address))
}
/// # Safety
/// The exact PTE must be removed and translations retired before this call.
/// No DMA or other aliases may still reference this exclusively mapped frame.
pub unsafe fn retire_mapping(owner: Owner, frame: u64, address: u64) -> Option<Grant> {
    with_memory(|_, grants| grants.retire(owner, frame, address))
}
pub fn owner_frames(owner: Owner) -> usize {
    with_memory(|_, grants| grants.owner_frames(owner))
}

pub fn allocated_frames() -> u64 {
    with_allocator(|allocator| allocator.allocated_frames())
}

#[allow(dead_code)]
pub fn recycled_frames() -> usize {
    with_allocator(|allocator| allocator.recycled_frames())
}

#[cfg(feature = "memory-test-faults")]
pub fn snapshot() -> (kernel::physmem::AllocatorStats, bool) {
    with_memory(|allocator, grants| {
        (
            allocator.stats(),
            allocator.is_consistent()
                && grants.is_consistent(allocator.allocated_frames(), |frame| {
                    allocator.contains_live(frame)
                }),
        )
    })
}

pub use kernel::memory_report::Report;

pub fn report() -> Result<Report, core::fmt::Error> {
    let (stats, consistent, live, kernel, denied, exhausted, quota_denials) =
        with_memory(|allocator, grants| {
            (
                allocator.stats(),
                allocator.is_consistent()
                    && grants.is_consistent(allocator.allocated_frames(), |frame| {
                        allocator.contains_live(frame)
                    }),
                grants.live(),
                grants.owner_frames(Owner::KERNEL),
                grants.denied,
                grants.exhausted,
                grants.quota_denials,
            )
        });
    Report::new(
        stats,
        consistent,
        kernel::memory_report::Grants {
            live,
            kernel,
            denied,
            exhausted,
            quota_denials,
        },
    )
}

#[cfg(feature = "memory-test-faults")]
pub fn run_hygiene_probe() -> bool {
    crate::arch::without_interrupts(|| {
        let baseline = allocated_frames();
        let Some(grant) = alloc_frame(Owner::KERNEL, Kind::User) else {
            return false;
        };
        let frame = grant.address();
        // SAFETY: validation owns the exact grant and its physical identity map.
        // IRQs stay masked throughout poisoning, release, inspection and reuse.
        // The post-release reads deliberately inspect the free page before any
        // allocator caller can acquire it, proving release-time scrubbing.
        let (initial_zero, poison_intact, erased) = unsafe {
            let initial_zero = (0..PAGE_SIZE)
                .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0);
            for offset in 0..PAGE_SIZE {
                core::ptr::write_volatile((frame + offset) as *mut u8, 0xa5);
            }
            let denied = find_grant(Owner::KERNEL, frame + 1, Kind::User).is_none()
                && find_grant(Owner::KERNEL, 0, Kind::User).is_none();
            let poison_intact = denied
                && (0..PAGE_SIZE)
                    .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0xa5);
            let erased = free_frame(grant)
                && (0..PAGE_SIZE)
                    .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0);
            (initial_zero, poison_intact, erased)
        };
        // SAFETY: this probe owns the original identity; a retired grant is
        // deliberately retried to check rejection before any physical access.
        let double_denied = unsafe { !free_frame(grant) };
        let Some(fresh) = alloc_frame(Owner::KERNEL, Kind::User) else {
            return false;
        };
        let reused = fresh.address();
        // SAFETY: this fresh identity owns the complete page. Check zeroing,
        // then poison it so stale release would observably damage its new owner.
        let (reuse_zero, stale_denied) = unsafe {
            let zero = (0..PAGE_SIZE)
                .all(|offset| core::ptr::read_volatile((reused + offset) as *const u8) == 0);
            core::ptr::write_bytes(reused as *mut u8, 0x5a, PAGE_SIZE as usize);
            let denied = !free_frame(grant)
                && (0..PAGE_SIZE)
                    .all(|offset| core::ptr::read_volatile((reused + offset) as *const u8) == 0x5a);
            (zero, denied)
        };
        // SAFETY: the new grant is owned only by this probe and is not mapped to a user.
        let returned = unsafe { free_frame(fresh) };
        initial_zero
            && poison_intact
            && erased
            && double_denied
            && stale_denied
            && reused == frame
            && reuse_zero
            && returned
            && allocated_frames() == baseline
            && snapshot().1
    })
}

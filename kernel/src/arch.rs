use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};
use kernel::boot_cpu::{BootClaim, Features as BootFeatures};
use kernel::stack::{GuardedStack, Layout as StackLayout, GUARD_BYTES};

// Entry can leave a user's floating-point state live while Rust handles the
// trap. The pinned bare-metal soft-float target must never consume that state.
#[cfg(any(
    target_feature = "sse",
    target_feature = "sse2",
    target_feature = "avx",
    target_feature = "avx512f"
))]
compile_error!("kernel Rust must use the x86_64-unknown-none soft-float target without SIMD");

static BOOT_CLAIM: BootClaim = BootClaim::new();
static STACK_GUARDS_READY: AtomicBool = AtomicBool::new(false);
const IA32_APIC_BASE: u32 = 0x1b;

/// First operation at the kernel entry point, before UART, tables or memory.
/// Rejected entrants must halt without accessing BSP-owned mutable state.
/// IF remains clear on both success and rejection.
pub fn claim_boot_cpu() -> bool {
    disable_interrupts();
    let (features, apic_base) = boot_cpu_identity();
    BOOT_CLAIM.claim(features, apic_base)
}

fn boot_cpu_identity() -> (BootFeatures, Option<u64>) {
    use core::arch::x86_64::__cpuid;
    // CPUID exists in long mode. Do not query a missing basic feature leaf.
    if __cpuid(0).eax < 1 {
        return (BootFeatures::from_cpuid(0, 0), None);
    }
    let leaf = __cpuid(1);
    let features = BootFeatures::from_cpuid(leaf.ebx, leaf.edx);
    if !features.can_read_apic_base() {
        return (features, None);
    }
    let (low, high): (u32, u32);
    // SAFETY: CPL0 x86_64 entry with a valid firmware-provided stack and IF
    // clear. CPUID reported both MSR and local APIC support, establishing the
    // architectural IA32_APIC_BASE read contract. No shared memory or APIC
    // state is mutated. EDX:EAX are outputs and ECX selects the register;
    // RDMSR leaves flags and stack intact and returns to the caller.
    unsafe {
        asm!("rdmsr", in("ecx") IA32_APIC_BASE, out("eax") low, out("edx") high,
             options(nomem, nostack, preserves_flags));
    }
    (features, Some((u64::from(high) << 32) | u64::from(low)))
}

// Reserved by the kernel ELF's BSS segment; the frame allocator never grants
// these pages to applications. Admission happens before this shared stack is used.
const BOOT_STACK_SIZE: usize = 2 * 1024 * 1024;
static mut BOOT_STACK: GuardedStack<BOOT_STACK_SIZE> = GuardedStack::new();

/// Transfer the admitted BSP from its small firmware entry frame to kernel storage.
///
/// # Safety
/// The caller must own the irreversible boot claim, have IF clear, and pass an
/// immutable boot-lifetime BootInfo. Firmware mappings must cover the loaded ELF.
/// No second entrant may use this stack. The SysV64 arguments arrive in RDI/RSI;
/// RDI survives unchanged and CALL establishes the required 16-byte alignment.
/// This transfer never returns and creates no references to the mutable stack.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn enter_boot_stack(
    _boot_info: &'static genos_abi::BootInfo,
    _entry: extern "sysv64" fn(&'static genos_abi::BootInfo) -> !,
) -> ! {
    core::arch::naked_asm!(
        // Fill before making the first kernel stack frame, preserving the two
        // SysV argument registers in scratch GPRs across REP STOSQ. The range
        // excludes both guard pages and remains reserved for this admitted BSP.
        "mov r8, rdi",
        "mov r9, rsi",
        "lea rdi, [rip + {stack}]",
        "add rdi, {guard}",
        "mov rcx, {words}",
        "mov rax, {watermark}",
        "cld",
        "rep stosq",
        "mov rsp, rdi",
        "mov rdi, r8",
        "xor ebp, ebp",
        "call r9",
        "ud2",
        stack = sym BOOT_STACK,
        guard = const GUARD_BYTES,
        words = const BOOT_STACK_SIZE / 8,
        watermark = const kernel::stack::WATERMARK_WORD,
    );
}

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x18;
pub const USER_DATA_SELECTOR: u16 = 0x2b;
pub const USER_CODE_SELECTOR: u16 = 0x33;
const INTERRUPT_IST_INDEX: u16 = 1;
const INTERRUPT_STACK_SIZE: usize = 64 * 1024;
const PRIVILEGE_STACK_SIZE: usize = 64 * 1024;
const EMERGENCY_STACK_SIZE: usize = 16 * 1024;

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
struct TaskStateSegment {
    reserved1: u32,
    rsp: [u64; 3],
    reserved2: u64,
    ist: [u64; 7],
    reserved3: u64,
    reserved4: u16,
    iomap_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            reserved1: 0,
            rsp: [0; 3],
            reserved2: 0,
            ist: [0; 7],
            reserved3: 0,
            reserved4: 0,
            iomap_base: core::mem::size_of::<TaskStateSegment>() as u16,
        }
    }
}

#[repr(C, align(4096))]
struct Idt([IdtEntry; 256]);

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    fn new(vector: usize, handler: unsafe extern "C" fn(), user_callable: bool) -> Self {
        let addr = handler as usize as u64;
        let ist = if user_callable {
            0
        } else {
            kernel::exception::ist_index(vector)
        };
        Self {
            offset_low: addr as u16,
            selector: KERNEL_CODE_SELECTOR,
            options: (if user_callable { 0xee00 } else { 0x8e00 }) | ist,
            offset_mid: (addr >> 16) as u16,
            offset_high: (addr >> 32) as u32,
            reserved: 0,
        }
    }
}

// The atomic BSP entry/table gates own these objects with IF clear. The IDT
// has its own page and becomes CPU-enforced read-only after IRQ installation.
static mut IDT: Idt = Idt([IdtEntry::missing(); 256]);
static mut GDT: [u64; 7] = [0; 7];
static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut INTERRUPT_STACK: GuardedStack<INTERRUPT_STACK_SIZE> = GuardedStack::new();
static mut PRIVILEGE_STACK: GuardedStack<PRIVILEGE_STACK_SIZE> = GuardedStack::new();
static mut DOUBLE_FAULT_STACK: GuardedStack<EMERGENCY_STACK_SIZE> = GuardedStack::new();
static mut NMI_STACK: GuardedStack<EMERGENCY_STACK_SIZE> = GuardedStack::new();
static mut MACHINE_CHECK_STACK: GuardedStack<EMERGENCY_STACK_SIZE> = GuardedStack::new();
static mut DEBUG_STACK: GuardedStack<EMERGENCY_STACK_SIZE> = GuardedStack::new();

/// These addresses describe reserved storage, without referencing its unmapped
/// guards. The ordering is also the bounded guard-proof fixture's stack index.
pub fn stack_regions() -> [(&'static str, StackLayout); 7] {
    [
        (
            "boot",
            core::ptr::addr_of!(BOOT_STACK) as u64,
            BOOT_STACK_SIZE,
        ),
        (
            "irq",
            core::ptr::addr_of!(INTERRUPT_STACK) as u64,
            INTERRUPT_STACK_SIZE,
        ),
        (
            "privilege",
            core::ptr::addr_of!(PRIVILEGE_STACK) as u64,
            PRIVILEGE_STACK_SIZE,
        ),
        (
            "double-fault",
            core::ptr::addr_of!(DOUBLE_FAULT_STACK) as u64,
            EMERGENCY_STACK_SIZE,
        ),
        (
            "nmi",
            core::ptr::addr_of!(NMI_STACK) as u64,
            EMERGENCY_STACK_SIZE,
        ),
        (
            "machine-check",
            core::ptr::addr_of!(MACHINE_CHECK_STACK) as u64,
            EMERGENCY_STACK_SIZE,
        ),
        (
            "debug",
            core::ptr::addr_of!(DEBUG_STACK) as u64,
            EMERGENCY_STACK_SIZE,
        ),
    ]
    .map(|(name, base, size)| {
        (
            name,
            StackLayout::new(base, size).expect("static stack geometry"),
        )
    })
}

/// Install all guards after the image's BSS was split into protected 4 KiB
/// mappings, before IRQ enable or construction of a process root.
pub fn install_stack_guards() -> Result<(), crate::paging::PagingError> {
    if interrupts_enabled() {
        return Err(crate::paging::PagingError::InvalidAddress);
    }
    let regions = stack_regions();
    let pointer: u64;
    // SAFETY: reading the current CPL0 RSP changes no flags, memory or stack.
    unsafe {
        asm!("mov {}, rsp", out(reg) pointer, options(nomem, nostack, preserves_flags));
    }
    if !regions[0].1.contains_stack_pointer(pointer)
        || regions.iter().enumerate().any(|(index, (_, region))| {
            regions[index + 1..]
                .iter()
                .any(|(_, other)| region.overlaps(*other))
        })
    {
        return Err(crate::paging::PagingError::InvalidAddress);
    }
    for (_, region) in regions {
        for guard in region.guards() {
            // SAFETY: the page-aligned static GuardedStack geometry dedicates
            // these pages to inaccessible guards, disjoint from every usable
            // stack and live object. BSP startup owns their mappings with IF
            // clear. No physical frame is released or offered for allocation.
            unsafe {
                crate::paging::guard_kernel_page(guard)?;
            }
        }
    }
    crate::serial::println("KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096");
    STACK_GUARDS_READY.store(true, Ordering::Release);
    Ok(())
}

/// Quiescent BSP measurements over usable storage only. No slice/reference is
/// created over live stack frames; assembly reads sample physical word bits.
pub fn stack_usage() -> Option<[(&'static str, usize, usize); 7]> {
    if !STACK_GUARDS_READY.load(Ordering::Acquire) {
        return None;
    }
    without_interrupts(|| {
        Some(stack_regions().map(|(name, region)| {
            let capacity = (region.top - region.usable_start) as usize;
            let used = kernel::stack::touched_bytes(capacity / 8, |index| {
                let value: u64;
                // SAFETY: this page-aligned usable range remains supervisor-
                // mapped and the index is bounded by capacity/8. The scan never
                // touches guards or creates typed reads/references to live Rust
                // locals, padding or moved values: MOV samples hardware bits
                // into a newly defined register output. IF is clear on the sole
                // BSP; NMI/fatal paths do not return and mutate this sample.
                unsafe {
                    asm!(
                        "mov {value}, qword ptr [{address}]",
                        address = in(reg) region.usable_start + index as u64 * 8,
                        value = out(reg) value,
                        options(nostack, readonly, preserves_flags),
                    );
                }
                value
            })
            .expect("bounded stack capacity");
            (name, capacity, used)
        }))
    })
}

/// Retain measurements and enforce a one-page observed margin in qualification
/// runs. Guard/probe policy supplies containment independently of this estimate.
pub fn report_stack_usage() -> bool {
    let Some(usage) = stack_usage() else {
        return false;
    };
    let mut margin = true;
    for (name, capacity, used) in usage {
        let remaining = capacity - used;
        crate::serial::print("KERNEL_STACK_USAGE name=");
        crate::serial::print(name);
        for (label, value) in [
            (" capacity=", capacity),
            (" touched=", used),
            (" remaining=", remaining),
        ] {
            crate::serial::print(label);
            crate::serial::print_u64(value as u64);
        }
        crate::serial::println("");
        margin &= remaining >= kernel::stack::MIN_REMAINING_BYTES;
    }
    margin
}

pub fn init() {
    disable_interrupts();
    let (features, apic_base) = boot_cpu_identity();
    if !BOOT_CLAIM.begin_table_init(features, apic_base) {
        halt_loop();
    }
    if !init_xstate() {
        crate::serial::println("CPU_XSTATE_UNSUPPORTED required=x87,mmx,fxsr,sse,sse2");
        halt_loop();
    }
    crate::serial::print("SMP_DISABLED policy=bsp-only active_cpus=1 initial_apic_id=");
    crate::serial::print_u64(u64::from(features.initial_apic_id));
    crate::serial::print(" cpuid_max_logical_per_package=");
    crate::serial::print_u64(u64::from(features.max_logical_per_package));
    crate::serial::println("");
    // SAFETY: the hardware BSP role and irreversible atomic table-init claim
    // above admit exactly one entrant. IF is clear. All table entries and
    // stack addresses are initialized before LIDT publishes the new table.
    unsafe {
        init_gdt();
        let idt_ptr = core::ptr::addr_of_mut!(IDT.0) as *mut IdtEntry;
        for index in 0..256 {
            idt_ptr.add(index).write(IdtEntry::new(
                index,
                crate::interrupts::vector_handler(index),
                false,
            ));
        }
        let ptr = DescriptorTablePointer {
            limit: (core::mem::size_of::<Idt>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
    crate::serial::println("IDT initialized");
}

fn init_xstate() -> bool {
    use core::arch::x86_64::__cpuid;
    use kernel::xstate;
    if __cpuid(0).eax < 1 || !xstate::supported(__cpuid(1).edx) {
        return false;
    }
    let (mut cr0, mut cr4): (u64, u64);
    let (mut low, high): (u32, u32);
    // SAFETY: the admitted BSP owns initialization with IF clear and no user
    // state yet. CPUID above establishes FXSAVE/SSE availability. CR0 disables
    // lazy #NM switching; CR4 enables SSE but forbids XSAVE-only components
    // and user PKRU/FS/GS base changes (PKE/FSGSBASE enable these independently
    // of OSXSAVE). General TLS and protection-key interfaces are not exposed.
    // Clear AMD's optional fast-FXSAVE bit, which can omit XMM at CPL0. EFER
    // exists in long mode, and all other bits remain unchanged. These writes
    // alter no memory mappings or stack state; readback gates publication.
    unsafe {
        asm!("mov {}, cr0", out(reg) cr0, options(nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nostack));
        cr0 = (cr0 | xstate::CR0_REQUIRED) & !xstate::CR0_FORBIDDEN;
        cr4 = (cr4 | xstate::CR4_REQUIRED)
            & !(xstate::CR4_OSXSAVE | xstate::CR4_PKE | xstate::CR4_FSGSBASE);
        asm!("mov cr0, {}", in(reg) cr0, options(nostack));
        asm!("mov cr4, {}", in(reg) cr4, options(nostack));
        asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") low, out("edx") high, options(nostack));
        low &= !(xstate::EFER_FFXSR as u32);
        asm!("wrmsr", in("ecx") 0xc000_0080u32, in("eax") low, in("edx") high, options(nostack));
        asm!("mov {}, cr0", out(reg) cr0, options(nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nostack));
    }
    let (low, high): (u32, u32);
    // SAFETY: same single-BSP EFER contract; this read verifies the write.
    unsafe {
        asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") low, out("edx") high, options(nostack));
    }
    if !xstate::configured(cr0, cr4, (u64::from(high) << 32) | u64::from(low)) {
        return false;
    }
    crate::serial::println(
        "CPU_XSTATE_READY mode=fxsave64 bytes=512 user=x87,mmx,sse,sse2 kernel=soft-float",
    );
    true
}

/// Called only after the supervisor-only page-table clone is active: enabling
/// SMEP on the firmware's inherited user mappings could fault kernel code.
pub fn init_page_protections() -> bool {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    use kernel::protection::{Features, CR0_WP, EFER_NXE};
    // CPUID is available on x86_64; unknown leaves are never queried.
    let features = {
        let extended = if __cpuid(0x8000_0000).eax >= 0x8000_0001 {
            __cpuid(0x8000_0001).edx
        } else {
            0
        };
        let structured = if __cpuid(0).eax >= 7 {
            __cpuid_count(7, 0).ebx
        } else {
            0
        };
        Features::from_cpuid(extended, structured)
    };
    if !features.nx {
        crate::serial::println("CPU_PROTECTIONS_UNSUPPORTED required=nx");
        return false;
    }
    let (mut low, high): (u32, u32);
    let (mut cr0, mut cr4): (u64, u64);
    // SAFETY: single BSP bootstrap, IF clear, supervisor page tables active.
    // Preserve unrelated control bits, enable only CPUID-supported features,
    // and verify the hardware state before publishing readiness.
    unsafe {
        asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") low, out("edx") high, options(nostack));
        low |= EFER_NXE as u32;
        asm!("wrmsr", in("ecx") 0xc000_0080u32, in("eax") low, in("edx") high, options(nostack));
        asm!("mov {}, cr0", out(reg) cr0, options(nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nostack));
        cr0 |= CR0_WP;
        cr4 |= features.cr4_bits();
        asm!("pushfq", "and qword ptr [rsp], -262145", "popfq");
        asm!("mov cr0, {}", in(reg) cr0, options(nostack));
        asm!("mov cr4, {}", in(reg) cr4, options(nostack));
        asm!("mov {}, cr0", out(reg) cr0, options(nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nostack));
    }
    let (efer_low, efer_high): (u32, u32);
    // SAFETY: EFER is present on every supported x86_64 CPU.
    unsafe {
        asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") efer_low, out("edx") efer_high, options(nostack));
    }
    if !features.verified((u64::from(efer_high) << 32) | u64::from(efer_low), cr0, cr4) {
        return false;
    }
    crate::serial::print("CPU_PROTECTIONS_READY nx=1 wp=1 smep=");
    crate::serial::print_u64(u64::from(features.smep));
    crate::serial::print(" smap=");
    crate::serial::print_u64(u64::from(features.smap));
    crate::serial::println("");
    true
}

pub fn idt_address() -> u64 {
    core::ptr::addr_of!(IDT) as u64
}

unsafe fn init_gdt() {
    let regions = stack_regions();
    for (_, region) in &regions[1..] {
        // SAFETY: these six stacks are disjoint and have never been published
        // in our TSS. IF is clear; the current boot stack was filled by naked
        // entry and is deliberately excluded from this initialization loop.
        core::ptr::write_bytes(
            region.usable_start as *mut u8,
            0xa5,
            (region.top - region.usable_start) as usize,
        );
    }
    TSS.ist[(INTERRUPT_IST_INDEX - 1) as usize] = regions[1].1.top;
    // SAFETY: static storage lives for the kernel lifetime, is disjoint, and
    // has 16-byte aligned tops. The processor owns each emergency stack on
    // entry; no normal Rust code creates references into these buffers.
    TSS.ist[1] = regions[3].1.top;
    TSS.ist[2] = regions[4].1.top;
    TSS.ist[3] = regions[5].1.top;
    TSS.ist[4] = regions[6].1.top;
    TSS.rsp[0] = regions[2].1.top;

    GDT[0] = 0;
    GDT[1] = 0x00af_9a00_0000_ffff;
    GDT[2] = 0x00cf_9200_0000_ffff;
    let (tss_low, tss_high) = tss_descriptor(core::ptr::addr_of!(TSS) as u64);
    GDT[3] = tss_low;
    GDT[4] = tss_high;
    GDT[5] = 0x00cf_f200_0000_ffff;
    GDT[6] = 0x00af_fa00_0000_ffff;

    let ptr = DescriptorTablePointer {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u64,
    };

    asm!(
        "lgdt [{gdt_ptr}]",
        "mov ax, {data}",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "push {code}",
        "lea rax, [rip + 2f]",
        "push rax",
        "retfq",
        "2:",
        "mov ax, {tss}",
        "ltr ax",
        gdt_ptr = in(reg) &ptr,
        code = const KERNEL_CODE_SELECTOR,
        data = const KERNEL_DATA_SELECTOR,
        tss = const TSS_SELECTOR,
        out("rax") _,
    );
    crate::serial::println("GDT/TSS initialized");
}

fn tss_descriptor(base: u64) -> (u64, u64) {
    let limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;
    let low = (limit & 0xffff)
        | ((base & 0x00ff_ffff) << 16)
        | (0x89u64 << 40)
        | (((limit >> 16) & 0x0f) << 48)
        | (((base >> 24) & 0xff) << 56);
    let high = base >> 32;
    (low, high)
}

pub unsafe fn set_idt_handler(vector: usize, handler: unsafe extern "C" fn()) {
    if vector < 256 {
        let idt_ptr = core::ptr::addr_of_mut!(IDT.0) as *mut IdtEntry;
        idt_ptr
            .add(vector)
            .write(IdtEntry::new(vector, handler, false));
    }
}

pub unsafe fn set_user_idt_handler(vector: usize, handler: unsafe extern "C" fn()) {
    if vector < 256 {
        let idt_ptr = core::ptr::addr_of_mut!(IDT.0) as *mut IdtEntry;
        idt_ptr
            .add(vector)
            .write(IdtEntry::new(vector, handler, true));
    }
}

pub fn enable_interrupts() {
    // SAFETY: CPL0 only; callers must install valid interrupt state first.
    // STI changes IF and preserves the stack. The implicit memory clobber
    // prevents protected memory operations moving past this boundary.
    unsafe { asm!("sti", options(nostack)) };
}

pub fn disable_interrupts() {
    // SAFETY: CPL0 only. CLI clears IF without changing the stack. The
    // implicit memory clobber prevents memory accesses escaping a critical
    // section. This masks local IRQs, not NMIs, exceptions or other CPUs.
    unsafe { asm!("cli", options(nostack)) };
}

/// Execute a bounded operation with local IRQs masked, restoring the caller's
/// IF state on every normal return. Nested sections never enable IRQs early.
/// Callers must not block or access this protected state from NMI/fatal handlers.
/// This is single-BSP serialization, not an SMP lock.
pub fn without_interrupts<R>(operation: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            if self.0 {
                enable_interrupts();
            }
        }
    }
    let enabled = interrupts_enabled();
    disable_interrupts();
    let _restore = Restore(enabled);
    operation()
}

pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe {
        asm!(
            "pushfq",
            "pop {}",
            out(reg) flags,
            options(preserves_flags),
        )
    };
    flags & (1 << 9) != 0
}

pub fn timestamp_cycles() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "lfence",
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

pub fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

#[allow(dead_code)]
// Retained for the legacy recovery console until ROADMAP F4 isolates that path.
pub fn reboot() -> ! {
    unsafe {
        loop {
            if inb(0x64) & 0x02 == 0 {
                outb(0x64, 0xfe);
            }
        }
    }
}

#[allow(dead_code)]
// Retained for the legacy recovery console until ROADMAP F4 isolates that path.
pub fn shutdown() -> ! {
    unsafe {
        outw(0x604, 0x2000);
        outw(0xb004, 0x2000);
    }
    halt_loop();
}

pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    asm!("in ax, dx", out("ax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

pub unsafe fn outw(port: u16, value: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
}

pub unsafe fn outl(port: u16, value: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
}

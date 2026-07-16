use core::arch::{asm, global_asm};

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x18;
pub const USER_DATA_SELECTOR: u16 = 0x2b;
pub const USER_CODE_SELECTOR: u16 = 0x33;
const INTERRUPT_IST_INDEX: u16 = 1;
const INTERRUPT_STACK_SIZE: usize = 64 * 1024;
const PRIVILEGE_STACK_SIZE: usize = 64 * 1024;

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

#[repr(C, align(16))]
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

    fn new(handler: unsafe extern "C" fn(), user_callable: bool) -> Self {
        let addr = handler as usize as u64;
        let ist = if user_callable {
            0
        } else {
            INTERRUPT_IST_INDEX
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

static mut IDT: Idt = Idt([IdtEntry::missing(); 256]);
static mut GDT: [u64; 7] = [0; 7];
static mut TSS: TaskStateSegment = TaskStateSegment::new();
#[repr(align(16))]
struct InterruptStack([u8; INTERRUPT_STACK_SIZE]);
static mut INTERRUPT_STACK: InterruptStack = InterruptStack([0; INTERRUPT_STACK_SIZE]);
#[repr(align(16))]
struct PrivilegeStack([u8; PRIVILEGE_STACK_SIZE]);
static mut PRIVILEGE_STACK: PrivilegeStack = PrivilegeStack([0; PRIVILEGE_STACK_SIZE]);

global_asm!(
    r#"
    .global genos_interrupt_stub
genos_interrupt_stub:
    iretq
"#
);

extern "C" {
    fn genos_interrupt_stub();
}

pub fn init() {
    unsafe {
        init_gdt();
        let idt_ptr = core::ptr::addr_of_mut!(IDT.0) as *mut IdtEntry;
        for index in 0..256 {
            idt_ptr
                .add(index)
                .write(IdtEntry::new(genos_interrupt_stub, false));
        }
        let ptr = DescriptorTablePointer {
            limit: (core::mem::size_of::<Idt>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
    crate::serial::println("IDT initialized");
}

unsafe fn init_gdt() {
    let stack_base = core::ptr::addr_of!(INTERRUPT_STACK.0) as u64;
    TSS.ist[(INTERRUPT_IST_INDEX - 1) as usize] = stack_base + INTERRUPT_STACK_SIZE as u64;
    let privilege_stack_base = core::ptr::addr_of!(PRIVILEGE_STACK.0) as u64;
    TSS.rsp[0] = privilege_stack_base + PRIVILEGE_STACK_SIZE as u64;

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
        idt_ptr.add(vector).write(IdtEntry::new(handler, false));
    }
}

pub unsafe fn set_user_idt_handler(vector: usize, handler: unsafe extern "C" fn()) {
    if vector < 256 {
        let idt_ptr = core::ptr::addr_of_mut!(IDT.0) as *mut IdtEntry;
        idt_ptr.add(vector).write(IdtEntry::new(handler, true));
    }
}

pub fn enable_interrupts() {
    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
}

pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
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

pub fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

pub fn reboot() -> ! {
    unsafe {
        loop {
            if inb(0x64) & 0x02 == 0 {
                outb(0x64, 0xfe);
            }
        }
    }
}

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

pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

pub unsafe fn outw(port: u16, value: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
}

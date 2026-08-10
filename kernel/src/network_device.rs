use core::{
    ptr::{addr_of, addr_of_mut, read_volatile, write_volatile},
    sync::atomic::{fence, Ordering},
};

use crate::{arch, serial};

pub const MAX_FRAME: usize = 1518;

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const PCI_VENDOR_VIRTIO: u16 = 0x1af4;
const PCI_DEVICE_VIRTIO_NET_TRANSITIONAL: u16 = 0x1000;
const PCI_DEVICE_VIRTIO_MODERN_MIN: u16 = 0x1040;
const PCI_DEVICE_VIRTIO_MODERN_MAX: u16 = 0x107f;
const PCI_CAP_VENDOR_SPECIFIC: u8 = 0x09;
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
const VIRTIO_STATUS_FAILED: u8 = 128;
const VIRTIO_NET_F_MAC: u32 = 1 << 5;
const VIRTIO_F_VERSION_1: u32 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
// VirtIO 1.x always includes the little-endian num_buffers field. The older
// 10-byte layout exists only for a legacy interface without mergeable buffers.
const VIRTIO_NET_HEADER_BYTES: usize = 12;
const VIRTQUEUE_SIZE: usize = 8;
const VIRTIO_BUFFER_BYTES: usize = 2048;
const VIRTIO_RX_QUEUE: u16 = 0;
const VIRTIO_TX_QUEUE: u16 = 1;
const VIRTIO_POLL_LIMIT: usize = 800_000;
const VIRTIO_RESET_POLL_LIMIT: usize = 100_000;

const COMMON_DEVICE_FEATURE_SELECT: usize = 0;
const COMMON_DEVICE_FEATURE: usize = 4;
const COMMON_DRIVER_FEATURE_SELECT: usize = 8;
const COMMON_DRIVER_FEATURE: usize = 12;
const COMMON_DEVICE_STATUS: usize = 20;
const COMMON_CONFIG_GENERATION: usize = 21;
const COMMON_QUEUE_SELECT: usize = 22;
const COMMON_QUEUE_SIZE: usize = 24;
const COMMON_QUEUE_MSIX_VECTOR: usize = 26;
const COMMON_QUEUE_ENABLE: usize = 28;
const COMMON_QUEUE_NOTIFY_OFF: usize = 30;
const COMMON_QUEUE_DESC: usize = 32;
const COMMON_QUEUE_DRIVER: usize = 40;
const COMMON_QUEUE_DEVICE: usize = 48;

const NE2000_IO_BASE: u16 = 0x300;
const NE2000_DATA_PORT: u16 = NE2000_IO_BASE + 0x10;
const NE2000_RESET_PORT: u16 = NE2000_IO_BASE + 0x1f;
const NE2000_TX_PAGE: u8 = 0x40;
const NE2000_RX_START: u8 = 0x46;
const NE2000_RX_STOP: u8 = 0x80;
const NE2000_POLL_LIMIT: usize = 800_000;

const NE2000_CR: u16 = NE2000_IO_BASE;
const NE2000_PSTART: u16 = NE2000_IO_BASE + 0x01;
const NE2000_PSTOP: u16 = NE2000_IO_BASE + 0x02;
const NE2000_BNRY: u16 = NE2000_IO_BASE + 0x03;
const NE2000_TPSR: u16 = NE2000_IO_BASE + 0x04;
const NE2000_TBCR0: u16 = NE2000_IO_BASE + 0x05;
const NE2000_TBCR1: u16 = NE2000_IO_BASE + 0x06;
const NE2000_ISR: u16 = NE2000_IO_BASE + 0x07;
const NE2000_RSAR0: u16 = NE2000_IO_BASE + 0x08;
const NE2000_RSAR1: u16 = NE2000_IO_BASE + 0x09;
const NE2000_RBCR0: u16 = NE2000_IO_BASE + 0x0a;
const NE2000_RBCR1: u16 = NE2000_IO_BASE + 0x0b;
const NE2000_RCR: u16 = NE2000_IO_BASE + 0x0c;
const NE2000_TCR: u16 = NE2000_IO_BASE + 0x0d;
const NE2000_DCR: u16 = NE2000_IO_BASE + 0x0e;
const NE2000_IMR: u16 = NE2000_IO_BASE + 0x0f;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PacketOwner {
    Free,
    Driver,
    Stack,
}

#[derive(Clone, Copy)]
pub struct PacketBuffer {
    pub bytes: [u8; MAX_FRAME],
    pub len: usize,
    pub owner: PacketOwner,
}

impl PacketBuffer {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; MAX_FRAME],
            len: 0,
            owner: PacketOwner::Free,
        }
    }
}

trait FrameDevice {
    fn init(&mut self) -> bool;
    fn mac(&self) -> [u8; 6];
    fn transmit(&mut self, frame: &[u8]) -> bool;
    fn receive(&mut self, packet: &mut PacketBuffer) -> bool;
}

pub enum NetworkDevice {
    Unavailable,
    Virtio(VirtioNet),
    Ne2000(Ne2000),
}

impl NetworkDevice {
    pub const fn new() -> Self {
        Self::Unavailable
    }

    pub fn discover(&mut self) -> bool {
        let mut virtio = VirtioNet::new();
        if virtio.init() {
            *self = Self::Virtio(virtio);
            return true;
        }
        let mut ne2000 = Ne2000::new();
        if ne2000.init() {
            *self = Self::Ne2000(ne2000);
            return true;
        }
        false
    }

    pub fn mac(&self) -> [u8; 6] {
        match self {
            Self::Virtio(device) => device.mac(),
            Self::Ne2000(device) => device.mac(),
            Self::Unavailable => [0; 6],
        }
    }

    pub fn transmit(&mut self, frame: &[u8]) -> bool {
        match self {
            Self::Virtio(device) => device.transmit(frame),
            Self::Ne2000(device) => device.transmit(frame),
            Self::Unavailable => false,
        }
    }

    pub fn receive(&mut self, packet: &mut PacketBuffer) -> bool {
        match self {
            Self::Virtio(device) => device.receive(packet),
            Self::Ne2000(device) => device.receive(packet),
            Self::Unavailable => false,
        }
    }

    pub fn driver_name(&self) -> &'static str {
        match self {
            Self::Virtio(_) => "virtio-net-pci",
            Self::Ne2000(_) => "ne2000-pio-legacy-fallback",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn transport_name(&self) -> &'static str {
        match self {
            Self::Virtio(_) => "modern-pci",
            Self::Ne2000(_) => "legacy-pio",
            Self::Unavailable => "none",
        }
    }

    pub fn receive_buffer_count(&self) -> usize {
        match self {
            Self::Virtio(_) => VIRTQUEUE_SIZE,
            Self::Ne2000(_) => 1,
            Self::Unavailable => 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDescriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

impl VirtqDescriptor {
    const fn empty() -> Self {
        Self {
            address: 0,
            length: 0,
            flags: 0,
            next: 0,
        }
    }
}

#[repr(C)]
struct VirtqAvailable {
    flags: u16,
    index: u16,
    ring: [u16; VIRTQUEUE_SIZE],
    used_event: u16,
}

impl VirtqAvailable {
    const fn empty() -> Self {
        Self {
            flags: 0,
            index: 0,
            ring: [0; VIRTQUEUE_SIZE],
            used_event: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqUsedElement {
    id: u32,
    length: u32,
}

impl VirtqUsedElement {
    const fn empty() -> Self {
        Self { id: 0, length: 0 }
    }
}

#[repr(C, align(4))]
struct VirtqUsed {
    flags: u16,
    index: u16,
    ring: [VirtqUsedElement; VIRTQUEUE_SIZE],
    available_event: u16,
}

impl VirtqUsed {
    const fn empty() -> Self {
        Self {
            flags: 0,
            index: 0,
            ring: [VirtqUsedElement::empty(); VIRTQUEUE_SIZE],
            available_event: 0,
        }
    }
}

#[repr(C, align(4096))]
struct VirtqueueMemory {
    descriptors: [VirtqDescriptor; VIRTQUEUE_SIZE],
    available: VirtqAvailable,
    used: VirtqUsed,
}

impl VirtqueueMemory {
    const fn empty() -> Self {
        Self {
            descriptors: [VirtqDescriptor::empty(); VIRTQUEUE_SIZE],
            available: VirtqAvailable::empty(),
            used: VirtqUsed::empty(),
        }
    }
}

#[repr(C, align(4096))]
struct VirtioRxBuffers([[u8; VIRTIO_BUFFER_BYTES]; VIRTQUEUE_SIZE]);

#[repr(C, align(4096))]
struct VirtioTxBuffer([u8; VIRTIO_BUFFER_BYTES]);

static mut VIRTIO_RX_QUEUE_MEMORY: VirtqueueMemory = VirtqueueMemory::empty();
static mut VIRTIO_TX_QUEUE_MEMORY: VirtqueueMemory = VirtqueueMemory::empty();
static mut VIRTIO_RX_BUFFERS: VirtioRxBuffers =
    VirtioRxBuffers([[0; VIRTIO_BUFFER_BYTES]; VIRTQUEUE_SIZE]);
static mut VIRTIO_TX_BUFFER: VirtioTxBuffer = VirtioTxBuffer([0; VIRTIO_BUFFER_BYTES]);

#[derive(Clone, Copy)]
struct PciLocation {
    bus: u8,
    device: u8,
    function: u8,
}

#[derive(Clone, Copy)]
struct VirtioPciRegions {
    common: u64,
    notify: u64,
    notify_multiplier: u32,
    device: u64,
}

pub struct VirtioNet {
    common: u64,
    rx_notify: u64,
    tx_notify: u64,
    mac: [u8; 6],
    rx_available: u16,
    rx_used: u16,
    tx_available: u16,
    tx_used: u16,
}

impl VirtioNet {
    const fn new() -> Self {
        Self {
            common: 0,
            rx_notify: 0,
            tx_notify: 0,
            mac: [0; 6],
            rx_available: 0,
            rx_used: 0,
            tx_available: 0,
            tx_used: 0,
        }
    }

    fn fail(&self) -> bool {
        if self.common != 0 {
            let status = unsafe { mmio_read_u8(self.common, COMMON_DEVICE_STATUS) };
            unsafe {
                mmio_write_u8(
                    self.common,
                    COMMON_DEVICE_STATUS,
                    status | VIRTIO_STATUS_FAILED,
                )
            };
        }
        false
    }

    unsafe fn configure_queue(
        &self,
        queue_index: u16,
        memory: *mut VirtqueueMemory,
    ) -> Option<u16> {
        mmio_write_u16(self.common, COMMON_QUEUE_SELECT, queue_index);
        let offered_size = mmio_read_u16(self.common, COMMON_QUEUE_SIZE);
        if offered_size < VIRTQUEUE_SIZE as u16
            || mmio_read_u16(self.common, COMMON_QUEUE_ENABLE) != 0
        {
            return None;
        }
        mmio_write_u16(self.common, COMMON_QUEUE_SIZE, VIRTQUEUE_SIZE as u16);
        mmio_write_u16(self.common, COMMON_QUEUE_MSIX_VECTOR, u16::MAX);
        mmio_write_u64(
            self.common,
            COMMON_QUEUE_DESC,
            addr_of_mut!((*memory).descriptors) as u64,
        );
        mmio_write_u64(
            self.common,
            COMMON_QUEUE_DRIVER,
            addr_of_mut!((*memory).available) as u64,
        );
        mmio_write_u64(
            self.common,
            COMMON_QUEUE_DEVICE,
            addr_of_mut!((*memory).used) as u64,
        );
        let notify_offset = mmio_read_u16(self.common, COMMON_QUEUE_NOTIFY_OFF);
        fence(Ordering::SeqCst);
        mmio_write_u16(self.common, COMMON_QUEUE_ENABLE, 1);
        Some(notify_offset)
    }

    unsafe fn prepare_receive_queue(&mut self) {
        let queue = addr_of_mut!(VIRTIO_RX_QUEUE_MEMORY);
        let buffers = addr_of_mut!(VIRTIO_RX_BUFFERS.0) as *mut [u8; VIRTIO_BUFFER_BYTES];
        for index in 0..VIRTQUEUE_SIZE {
            write_volatile(
                addr_of_mut!((*queue).descriptors[index]),
                VirtqDescriptor {
                    address: buffers.add(index) as u64,
                    length: VIRTIO_BUFFER_BYTES as u32,
                    flags: VIRTQ_DESC_F_WRITE,
                    next: 0,
                },
            );
            write_volatile(addr_of_mut!((*queue).available.ring[index]), index as u16);
        }
        fence(Ordering::Release);
        write_volatile(
            addr_of_mut!((*queue).available.index),
            VIRTQUEUE_SIZE as u16,
        );
        self.rx_available = VIRTQUEUE_SIZE as u16;
        self.rx_used = 0;
    }

    unsafe fn notify(address: u64, queue_index: u16) {
        fence(Ordering::SeqCst);
        write_volatile(address as *mut u16, queue_index);
    }
}

impl FrameDevice for VirtioNet {
    fn init(&mut self) -> bool {
        let Some(location) = discover_virtio_net() else {
            return false;
        };
        let Some(regions) = discover_virtio_regions(location) else {
            return false;
        };
        self.common = regions.common;

        unsafe {
            mmio_write_u8(self.common, COMMON_DEVICE_STATUS, 0);
            let mut reset = false;
            for _ in 0..VIRTIO_RESET_POLL_LIMIT {
                if mmio_read_u8(self.common, COMMON_DEVICE_STATUS) == 0 {
                    reset = true;
                    break;
                }
            }
            if !reset {
                return self.fail();
            }
            mmio_write_u8(
                self.common,
                COMMON_DEVICE_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER,
            );

            mmio_write_u32(self.common, COMMON_DEVICE_FEATURE_SELECT, 0);
            let features_low = mmio_read_u32(self.common, COMMON_DEVICE_FEATURE);
            mmio_write_u32(self.common, COMMON_DEVICE_FEATURE_SELECT, 1);
            let features_high = mmio_read_u32(self.common, COMMON_DEVICE_FEATURE);
            if features_low & VIRTIO_NET_F_MAC == 0 || features_high & VIRTIO_F_VERSION_1 == 0 {
                return self.fail();
            }

            mmio_write_u32(self.common, COMMON_DRIVER_FEATURE_SELECT, 0);
            mmio_write_u32(self.common, COMMON_DRIVER_FEATURE, VIRTIO_NET_F_MAC);
            mmio_write_u32(self.common, COMMON_DRIVER_FEATURE_SELECT, 1);
            mmio_write_u32(self.common, COMMON_DRIVER_FEATURE, VIRTIO_F_VERSION_1);
            let mut status =
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK;
            mmio_write_u8(self.common, COMMON_DEVICE_STATUS, status);
            if mmio_read_u8(self.common, COMMON_DEVICE_STATUS) & VIRTIO_STATUS_FEATURES_OK == 0 {
                return self.fail();
            }

            let mut stable_config = false;
            for _ in 0..3 {
                let generation = mmio_read_u8(self.common, COMMON_CONFIG_GENERATION);
                for index in 0..self.mac.len() {
                    self.mac[index] = read_volatile((regions.device + index as u64) as *const u8);
                }
                if generation == mmio_read_u8(self.common, COMMON_CONFIG_GENERATION) {
                    stable_config = true;
                    break;
                }
            }
            if !stable_config || self.mac == [0; 6] || self.mac == [0xff; 6] {
                return self.fail();
            }
            serial::println("VIRTIO_NET_FEATURES_OK version=1 mac=true offloads=false");

            write_volatile(
                addr_of_mut!(VIRTIO_RX_QUEUE_MEMORY),
                VirtqueueMemory::empty(),
            );
            write_volatile(
                addr_of_mut!(VIRTIO_TX_QUEUE_MEMORY),
                VirtqueueMemory::empty(),
            );
            let Some(rx_notify_offset) =
                self.configure_queue(VIRTIO_RX_QUEUE, addr_of_mut!(VIRTIO_RX_QUEUE_MEMORY))
            else {
                return self.fail();
            };
            let Some(tx_notify_offset) =
                self.configure_queue(VIRTIO_TX_QUEUE, addr_of_mut!(VIRTIO_TX_QUEUE_MEMORY))
            else {
                return self.fail();
            };
            self.rx_notify = regions
                .notify
                .checked_add(u64::from(rx_notify_offset) * u64::from(regions.notify_multiplier))
                .unwrap_or(0);
            self.tx_notify = regions
                .notify
                .checked_add(u64::from(tx_notify_offset) * u64::from(regions.notify_multiplier))
                .unwrap_or(0);
            if self.rx_notify == 0 || self.tx_notify == 0 {
                return self.fail();
            }

            self.prepare_receive_queue();
            self.tx_available = 0;
            self.tx_used = 0;
            status |= VIRTIO_STATUS_DRIVER_OK;
            mmio_write_u8(self.common, COMMON_DEVICE_STATUS, status);
            Self::notify(self.rx_notify, VIRTIO_RX_QUEUE);
            serial::println("VIRTIO_NET_QUEUES_READY rx=8 tx=8 layout=split");
        }
        true
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn transmit(&mut self, frame: &[u8]) -> bool {
        if frame.len() > MAX_FRAME {
            return false;
        }
        let wire_length = frame.len().max(60);
        let total_length = VIRTIO_NET_HEADER_BYTES + wire_length;
        if total_length > VIRTIO_BUFFER_BYTES {
            return false;
        }
        unsafe {
            let queue = addr_of_mut!(VIRTIO_TX_QUEUE_MEMORY);
            let buffer = addr_of_mut!(VIRTIO_TX_BUFFER.0) as *mut u8;
            core::ptr::write_bytes(buffer, 0, total_length);
            core::ptr::copy_nonoverlapping(
                frame.as_ptr(),
                buffer.add(VIRTIO_NET_HEADER_BYTES),
                frame.len(),
            );
            write_volatile(
                addr_of_mut!((*queue).descriptors[0]),
                VirtqDescriptor {
                    address: buffer as u64,
                    length: total_length as u32,
                    flags: 0,
                    next: 0,
                },
            );
            write_volatile(
                addr_of_mut!((*queue).available.ring[self.tx_available as usize % VIRTQUEUE_SIZE]),
                0,
            );
            fence(Ordering::Release);
            self.tx_available = self.tx_available.wrapping_add(1);
            write_volatile(addr_of_mut!((*queue).available.index), self.tx_available);
            Self::notify(self.tx_notify, VIRTIO_TX_QUEUE);
            for _ in 0..VIRTIO_POLL_LIMIT {
                fence(Ordering::Acquire);
                let used = read_volatile(addr_of!((*queue).used.index));
                if used != self.tx_used {
                    let element = read_volatile(addr_of!(
                        (*queue).used.ring[self.tx_used as usize % VIRTQUEUE_SIZE]
                    ));
                    self.tx_used = self.tx_used.wrapping_add(1);
                    if element.id != 0 {
                        serial::println("VIRTIO_NET_TX_INVALID_USED_ID");
                    }
                    return element.id == 0;
                }
            }
        }
        serial::println("VIRTIO_NET_TX_TIMEOUT");
        false
    }

    fn receive(&mut self, packet: &mut PacketBuffer) -> bool {
        unsafe {
            let queue = addr_of_mut!(VIRTIO_RX_QUEUE_MEMORY);
            fence(Ordering::Acquire);
            let used_index = read_volatile(addr_of!((*queue).used.index));
            if self.rx_used == used_index {
                return false;
            }
            let element = read_volatile(addr_of!(
                (*queue).used.ring[self.rx_used as usize % VIRTQUEUE_SIZE]
            ));
            self.rx_used = self.rx_used.wrapping_add(1);
            let descriptor = element.id as usize;
            let valid = descriptor < VIRTQUEUE_SIZE
                && element.length as usize >= VIRTIO_NET_HEADER_BYTES
                && element.length as usize <= VIRTIO_BUFFER_BYTES;
            let frame_length = if valid {
                element.length as usize - VIRTIO_NET_HEADER_BYTES
            } else {
                0
            };
            if !valid || frame_length > MAX_FRAME {
                serial::println("VIRTIO_NET_RX_INVALID_BUFFER");
            }
            if valid && frame_length <= MAX_FRAME {
                packet.owner = PacketOwner::Driver;
                let buffers = addr_of!(VIRTIO_RX_BUFFERS.0) as *const [u8; VIRTIO_BUFFER_BYTES];
                core::ptr::copy_nonoverlapping(
                    buffers
                        .add(descriptor)
                        .cast::<u8>()
                        .add(VIRTIO_NET_HEADER_BYTES),
                    packet.bytes.as_mut_ptr(),
                    frame_length,
                );
                packet.len = frame_length;
                packet.owner = PacketOwner::Stack;
            }

            if descriptor < VIRTQUEUE_SIZE {
                write_volatile(
                    addr_of_mut!(
                        (*queue).available.ring[self.rx_available as usize % VIRTQUEUE_SIZE]
                    ),
                    descriptor as u16,
                );
                fence(Ordering::Release);
                self.rx_available = self.rx_available.wrapping_add(1);
                write_volatile(addr_of_mut!((*queue).available.index), self.rx_available);
                Self::notify(self.rx_notify, VIRTIO_RX_QUEUE);
            }
            valid && frame_length <= MAX_FRAME
        }
    }
}

fn discover_virtio_net() -> Option<PciLocation> {
    for bus in 0u16..=255 {
        for device in 0u8..32 {
            for function in 0u8..8 {
                let location = PciLocation {
                    bus: bus as u8,
                    device,
                    function,
                };
                let id = pci_read_u32(location, 0);
                if id as u16 != PCI_VENDOR_VIRTIO {
                    continue;
                }
                let device_id = (id >> 16) as u16;
                if device_id != PCI_DEVICE_VIRTIO_NET_TRANSITIONAL
                    && !(PCI_DEVICE_VIRTIO_MODERN_MIN..=PCI_DEVICE_VIRTIO_MODERN_MAX)
                        .contains(&device_id)
                {
                    continue;
                }
                if pci_read_u8(location, 0x0b) != 0x02 {
                    continue;
                }
                let command = pci_read_u32(location, 0x04) & 0x0000_ffff;
                pci_write_u32(location, 0x04, command | 0x0000_0006);
                return Some(location);
            }
        }
    }
    None
}

fn discover_virtio_regions(location: PciLocation) -> Option<VirtioPciRegions> {
    if pci_read_u16(location, 0x06) & 0x10 == 0 {
        return None;
    }
    let mut common = None;
    let mut notify = None;
    let mut notify_multiplier = 0;
    let mut device = None;
    let mut capability = pci_read_u8(location, 0x34) & 0xfc;
    for _ in 0..48 {
        if capability < 0x40 {
            break;
        }
        let next = pci_read_u8(location, capability.wrapping_add(1)) & 0xfc;
        if pci_read_u8(location, capability) == PCI_CAP_VENDOR_SPECIFIC {
            let length = pci_read_u8(location, capability.wrapping_add(2));
            let kind = pci_read_u8(location, capability.wrapping_add(3));
            if length >= 16 {
                let bar = pci_read_u8(location, capability.wrapping_add(4));
                let offset = pci_read_u32(location, capability.wrapping_add(8));
                let region_length = pci_read_u32(location, capability.wrapping_add(12));
                if region_length != 0 {
                    if let Some(base) = pci_bar_base(location, bar)
                        .and_then(|base| base.checked_add(u64::from(offset)))
                    {
                        match kind {
                            VIRTIO_PCI_CAP_COMMON_CFG if region_length >= 56 => common = Some(base),
                            VIRTIO_PCI_CAP_NOTIFY_CFG if length >= 20 => {
                                notify = Some(base);
                                notify_multiplier =
                                    pci_read_u32(location, capability.wrapping_add(16));
                            }
                            VIRTIO_PCI_CAP_DEVICE_CFG if region_length >= 6 => device = Some(base),
                            _ => {}
                        }
                    }
                }
            }
        }
        if next == 0 || next == capability {
            break;
        }
        capability = next;
    }
    let regions = VirtioPciRegions {
        common: common?,
        notify: notify?,
        notify_multiplier,
        device: device?,
    };
    (regions.notify_multiplier != 0).then_some(regions)
}

fn pci_bar_base(location: PciLocation, bar: u8) -> Option<u64> {
    if bar >= 6 {
        return None;
    }
    let offset = 0x10u8.checked_add(bar.checked_mul(4)?)?;
    let low = pci_read_u32(location, offset);
    if low & 1 != 0 {
        return None;
    }
    let memory_type = (low >> 1) & 0x03;
    let mut base = u64::from(low & !0x0f);
    if memory_type == 0x02 {
        if bar >= 5 {
            return None;
        }
        base |= u64::from(pci_read_u32(location, offset + 4)) << 32;
    } else if memory_type != 0 {
        return None;
    }
    (base != 0).then_some(base)
}

fn pci_read_u8(location: PciLocation, offset: u8) -> u8 {
    (pci_read_u32(location, offset) >> (u32::from(offset & 3) * 8)) as u8
}

fn pci_read_u16(location: PciLocation, offset: u8) -> u16 {
    (pci_read_u32(location, offset) >> (u32::from(offset & 2) * 8)) as u16
}

fn pci_read_u32(location: PciLocation, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | u32::from(location.bus) << 16
        | u32::from(location.device) << 11
        | u32::from(location.function) << 8
        | u32::from(offset & 0xfc);
    unsafe {
        arch::outl(PCI_CONFIG_ADDRESS, address);
        arch::inl(PCI_CONFIG_DATA)
    }
}

fn pci_write_u32(location: PciLocation, offset: u8, value: u32) {
    let address = 0x8000_0000u32
        | u32::from(location.bus) << 16
        | u32::from(location.device) << 11
        | u32::from(location.function) << 8
        | u32::from(offset & 0xfc);
    unsafe {
        arch::outl(PCI_CONFIG_ADDRESS, address);
        arch::outl(PCI_CONFIG_DATA, value);
    }
}

unsafe fn mmio_read_u8(base: u64, offset: usize) -> u8 {
    read_volatile((base + offset as u64) as *const u8)
}

unsafe fn mmio_read_u16(base: u64, offset: usize) -> u16 {
    read_volatile((base + offset as u64) as *const u16)
}

unsafe fn mmio_read_u32(base: u64, offset: usize) -> u32 {
    read_volatile((base + offset as u64) as *const u32)
}

unsafe fn mmio_write_u8(base: u64, offset: usize, value: u8) {
    write_volatile((base + offset as u64) as *mut u8, value);
}

unsafe fn mmio_write_u16(base: u64, offset: usize, value: u16) {
    write_volatile((base + offset as u64) as *mut u16, value);
}

unsafe fn mmio_write_u32(base: u64, offset: usize, value: u32) {
    write_volatile((base + offset as u64) as *mut u32, value);
}

unsafe fn mmio_write_u64(base: u64, offset: usize, value: u64) {
    write_volatile((base + offset as u64) as *mut u64, value);
}

pub struct Ne2000 {
    mac: [u8; 6],
    next_rx: u8,
}

impl Ne2000 {
    const fn new() -> Self {
        Self {
            mac: [0; 6],
            next_rx: NE2000_RX_START + 1,
        }
    }

    fn ring_read(&mut self, address: u16, output: &mut [u8]) -> bool {
        let ring_end = u16::from(NE2000_RX_STOP) << 8;
        if address + output.len() as u16 <= ring_end {
            return self.remote_read(address, output);
        }
        let first = usize::from(ring_end - address);
        self.remote_read(address, &mut output[..first])
            && self.remote_read(u16::from(NE2000_RX_START) << 8, &mut output[first..])
    }

    fn remote_read(&mut self, address: u16, output: &mut [u8]) -> bool {
        let count = (output.len() + 1) & !1;
        ne2000_write(NE2000_CR, 0x22);
        ne2000_write(NE2000_RBCR0, count as u8);
        ne2000_write(NE2000_RBCR1, (count >> 8) as u8);
        ne2000_write(NE2000_RSAR0, address as u8);
        ne2000_write(NE2000_RSAR1, (address >> 8) as u8);
        ne2000_write(NE2000_ISR, 0x40);
        ne2000_write(NE2000_CR, 0x0a);
        for index in (0..count).step_by(2) {
            let word = unsafe { arch::inw(NE2000_DATA_PORT) }.to_le_bytes();
            if index < output.len() {
                output[index] = word[0];
            }
            if index + 1 < output.len() {
                output[index + 1] = word[1];
            }
        }
        ne2000_wait_register(NE2000_ISR, 0x40)
    }

    fn remote_write(&mut self, address: u16, input: &[u8]) -> bool {
        let count = (input.len() + 1) & !1;
        ne2000_write(NE2000_CR, 0x22);
        ne2000_write(NE2000_RBCR0, count as u8);
        ne2000_write(NE2000_RBCR1, (count >> 8) as u8);
        ne2000_write(NE2000_RSAR0, address as u8);
        ne2000_write(NE2000_RSAR1, (address >> 8) as u8);
        ne2000_write(NE2000_ISR, 0x40);
        ne2000_write(NE2000_CR, 0x12);
        for index in (0..count).step_by(2) {
            let low = input.get(index).copied().unwrap_or(0);
            let high = input.get(index + 1).copied().unwrap_or(0);
            unsafe { arch::outw(NE2000_DATA_PORT, u16::from_le_bytes([low, high])) };
        }
        ne2000_wait_register(NE2000_ISR, 0x40)
    }

    fn reset_ring(&mut self, current: u8) {
        self.next_rx = if (NE2000_RX_START..NE2000_RX_STOP).contains(&current) {
            current
        } else {
            NE2000_RX_START + 1
        };
        let boundary = if self.next_rx == NE2000_RX_START {
            NE2000_RX_STOP - 1
        } else {
            self.next_rx - 1
        };
        ne2000_write(NE2000_BNRY, boundary);
    }
}

impl FrameDevice for Ne2000 {
    fn init(&mut self) -> bool {
        let reset = unsafe { arch::inb(NE2000_RESET_PORT) };
        if reset == 0xff {
            return false;
        }
        unsafe { arch::outb(NE2000_RESET_PORT, reset) };
        if !ne2000_wait_register(NE2000_ISR, 0x80) {
            return false;
        }
        ne2000_write(NE2000_CR, 0x21);
        ne2000_write(NE2000_DCR, 0x49);
        ne2000_write(NE2000_RBCR0, 0);
        ne2000_write(NE2000_RBCR1, 0);
        ne2000_write(NE2000_RCR, 0x20);
        ne2000_write(NE2000_TCR, 0x02);
        ne2000_write(NE2000_PSTART, NE2000_RX_START);
        ne2000_write(NE2000_BNRY, NE2000_RX_START);
        ne2000_write(NE2000_PSTOP, NE2000_RX_STOP);
        ne2000_write(NE2000_ISR, 0xff);
        ne2000_write(NE2000_IMR, 0);
        let mut prom = [0u8; 32];
        if !self.remote_read(0, &mut prom) {
            return false;
        }
        for (index, byte) in self.mac.iter_mut().enumerate() {
            *byte = prom[index * 2];
        }
        if self.mac == [0; 6] || self.mac == [0xff; 6] {
            return false;
        }
        ne2000_write(NE2000_CR, 0x61);
        for (index, byte) in self.mac.iter().copied().enumerate() {
            ne2000_write(NE2000_IO_BASE + 1 + index as u16, byte);
        }
        ne2000_write(NE2000_IO_BASE + 7, self.next_rx);
        for offset in 8..=15 {
            ne2000_write(NE2000_IO_BASE + offset, 0);
        }
        ne2000_write(NE2000_CR, 0x22);
        ne2000_write(NE2000_TCR, 0);
        ne2000_write(NE2000_RCR, 0x04);
        ne2000_write(NE2000_ISR, 0xff);
        true
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn transmit(&mut self, frame: &[u8]) -> bool {
        if frame.len() > MAX_FRAME {
            return false;
        }
        let wire_len = frame.len().max(60);
        let mut padded = [0u8; MAX_FRAME];
        padded[..frame.len()].copy_from_slice(frame);
        if !self.remote_write(u16::from(NE2000_TX_PAGE) << 8, &padded[..wire_len]) {
            return false;
        }
        ne2000_write(NE2000_TPSR, NE2000_TX_PAGE);
        ne2000_write(NE2000_TBCR0, wire_len as u8);
        ne2000_write(NE2000_TBCR1, (wire_len >> 8) as u8);
        ne2000_write(NE2000_ISR, 0x0a);
        ne2000_write(NE2000_CR, 0x26);
        for _ in 0..NE2000_POLL_LIMIT {
            let status = ne2000_read(NE2000_ISR);
            if status & 0x02 != 0 {
                ne2000_write(NE2000_ISR, 0x02);
                return true;
            }
            if status & 0x08 != 0 {
                ne2000_write(NE2000_ISR, 0x08);
                return false;
            }
        }
        false
    }

    fn receive(&mut self, packet: &mut PacketBuffer) -> bool {
        ne2000_write(NE2000_CR, 0x62);
        let current = ne2000_read(NE2000_IO_BASE + 7);
        ne2000_write(NE2000_CR, 0x22);
        if self.next_rx == current {
            return false;
        }
        let mut header = [0u8; 4];
        if !self.ring_read(u16::from(self.next_rx) << 8, &mut header) {
            return false;
        }
        let next = header[1];
        let recorded = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if header[0] & 0x01 == 0
            || !(NE2000_RX_START..NE2000_RX_STOP).contains(&next)
            || !(4..=MAX_FRAME + 4).contains(&recorded)
        {
            self.reset_ring(current);
            return false;
        }
        let len = recorded - 4;
        packet.owner = PacketOwner::Driver;
        if !self.ring_read((u16::from(self.next_rx) << 8) + 4, &mut packet.bytes[..len]) {
            packet.owner = PacketOwner::Free;
            return false;
        }
        packet.len = len;
        packet.owner = PacketOwner::Stack;
        self.next_rx = next;
        let boundary = if next == NE2000_RX_START {
            NE2000_RX_STOP - 1
        } else {
            next - 1
        };
        ne2000_write(NE2000_BNRY, boundary);
        true
    }
}

fn ne2000_write(port: u16, value: u8) {
    unsafe { arch::outb(port, value) };
}

fn ne2000_read(port: u16) -> u8 {
    unsafe { arch::inb(port) }
}

fn ne2000_wait_register(port: u16, mask: u8) -> bool {
    for _ in 0..NE2000_POLL_LIMIT {
        if ne2000_read(port) & mask != 0 {
            return true;
        }
    }
    false
}

#![no_std]

#[cfg(test)]
extern crate std;

pub mod capability;
pub mod display;
pub mod elf;
pub mod exception;
pub mod input;
pub mod ipc;
pub mod ipv6;
pub mod net;
pub mod page_table;
pub mod physmem;
pub mod protection;
pub mod recovery;
pub mod request;
pub mod serial_transport;
pub mod socket;
pub mod syscall;
pub mod tasks;
pub mod vfs;

#[cfg(test)]
extern crate self as kernel;
#[cfg(test)]
#[path = "network_transport_tests.rs"]
mod network_transport;
#[cfg(test)]
use network_transport::{network_device, serial};

#[cfg(test)]
mod arch {
    pub unsafe fn inb(_: u16) -> u8 {
        panic!("unexpected hardware I/O in driver test")
    }
    pub unsafe fn inw(_: u16) -> u16 {
        panic!("unexpected hardware I/O in driver test")
    }
    pub unsafe fn inl(_: u16) -> u32 {
        panic!("unexpected hardware I/O in driver test")
    }
    pub unsafe fn outb(_: u16, _: u8) {
        panic!("unexpected hardware I/O in driver test")
    }
    pub unsafe fn outw(_: u16, _: u16) {
        panic!("unexpected hardware I/O in driver test")
    }
    pub unsafe fn outl(_: u16, _: u32) {
        panic!("unexpected hardware I/O in driver test")
    }
}
#[cfg(test)]
#[allow(dead_code)] // Compile the production driver; only RX completion is invoked.
#[path = "network_device.rs"]
mod network_device_under_test;

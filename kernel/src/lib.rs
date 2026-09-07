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

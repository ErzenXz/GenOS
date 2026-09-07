//! Exercise the production packet/state-machine code with an in-memory frame
//! device. No host sockets or test reimplementation of TCP are involved.
#![allow(dead_code)]

pub(crate) mod serial {
    pub fn print(_: &str) {}
    pub fn println(_: &str) {}
    pub fn print_u64(_: u64) {}
}

pub(crate) mod network_device {
    use std::collections::VecDeque;
    use std::vec::Vec;

    pub const MAX_FRAME: usize = 1518;
    pub const VIRTIO_QUEUE_CAPACITY: usize = 8;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum PacketOwner {
        Free,
        Driver,
        Stack,
    }

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

    pub struct NetworkDevice {
        pub incoming: VecDeque<Vec<u8>>,
        pub outgoing: Vec<Vec<u8>>,
        pub fail_transmit: bool,
    }

    impl NetworkDevice {
        pub const fn new() -> Self {
            Self {
                incoming: VecDeque::new(),
                outgoing: Vec::new(),
                fail_transmit: false,
            }
        }
        pub fn discover(&mut self) -> bool {
            true
        }
        pub fn mac(&self) -> [u8; 6] {
            [0x52, 0x54, 0, 0x12, 0x34, 0x56]
        }
        pub fn driver_name(&self) -> &'static str {
            "memory-test"
        }
        pub fn transport_name(&self) -> &'static str {
            "frames"
        }
        pub fn receive_buffer_count(&self) -> usize {
            8
        }
        pub fn transmit(&mut self, bytes: &[u8]) -> bool {
            if self.fail_transmit {
                return false;
            }
            assert!(bytes.len() <= MAX_FRAME);
            self.outgoing.push(bytes.to_vec());
            true
        }
        pub fn receive(&mut self, packet: &mut PacketBuffer) -> bool {
            let Some(frame) = self.incoming.pop_front() else {
                return false;
            };
            assert!(frame.len() <= MAX_FRAME);
            packet.bytes[..frame.len()].copy_from_slice(&frame);
            packet.len = frame.len();
            packet.owner = PacketOwner::Stack;
            true
        }
    }

    #[derive(Default)]
    pub struct Metrics {
        pub rx_frames: u64,
        pub tx_frames: u64,
        pub rx_interrupt_completions: u64,
        pub tx_interrupt_completions: u64,
        pub interrupts: u64,
        pub recovery_completions: u64,
        pub recovery_polls: u64,
        pub queue_notifications: u64,
    }
    pub fn metrics() -> Metrics {
        Metrics::default()
    }
}

#[path = "network.rs"]
mod network;

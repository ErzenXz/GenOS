use core::fmt::{self, Write};

use crate::arch::{inb, outb};

const COM1: u16 = 0x3f8;

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xc7);
        outb(COM1 + 4, 0x0b);
    }
}

pub fn print(text: &str) {
    let _ = Serial.write_str(text);
}

pub fn println(text: &str) {
    print(text);
    print("\n");
}

pub fn print_u64(mut value: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    if value == 0 {
        write_byte(b'0');
        return;
    }
    while value > 0 {
        i -= 1;
        buf[i] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    for byte in &buf[i..] {
        if !write_byte(*byte) {
            break;
        }
    }
}

pub fn print_hex(mut value: u64) {
    let mut buf = [0u8; 16];
    let mut i = buf.len();
    if value == 0 {
        write_byte(b'0');
        return;
    }
    while value > 0 {
        i -= 1;
        let digit = (value & 0xf) as u8;
        buf[i] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        value >>= 4;
    }
    for byte in &buf[i..] {
        if !write_byte(*byte) {
            break;
        }
    }
}

pub fn read_byte() -> Option<u8> {
    unsafe { (inb(COM1 + 5) & 1 != 0).then(|| inb(COM1)) }
}

pub fn echo_byte(byte: u8) {
    write_byte(byte);
}

struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if !write_byte(byte) {
                return Err(fmt::Error);
            }
        }
        Ok(())
    }
}

fn write_byte(byte: u8) -> bool {
    // SAFETY: COM1 status reads and writes are bounded port I/O. A failed
    // transmitter drops output rather than blocking containment or halt.
    unsafe {
        if !kernel::serial_transport::wait_ready(|| inb(COM1 + 5) & 0x20 != 0) {
            return false;
        }
        outb(COM1, byte);
    }
    true
}

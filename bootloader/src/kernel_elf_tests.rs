extern crate std;

use super::*;
use std::{vec, vec::Vec};

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn segment(
    bytes: &mut [u8],
    index: usize,
    flags: u32,
    offset: u64,
    address: u64,
    file_size: u64,
    memory_size: u64,
) {
    let at = HEADER_BYTES + index * PROGRAM_HEADER_BYTES;
    put32(bytes, at, PT_LOAD);
    put32(bytes, at + 4, flags);
    for (field, value) in [
        (8, offset),
        (16, address),
        (24, address),
        (32, file_size),
        (40, memory_size),
        (48, 4096),
    ] {
        put64(bytes, at + field, value);
    }
}

fn valid() -> Vec<u8> {
    let mut bytes = vec![0; 0x2010];
    bytes[..9].copy_from_slice(b"\x7fELF\x02\x01\x01\x00\x00");
    put16(&mut bytes, 16, 2);
    put16(&mut bytes, 18, 62);
    put32(&mut bytes, 20, 1);
    put64(&mut bytes, 24, KERNEL_BASE + 8);
    put64(&mut bytes, 32, 64);
    put16(&mut bytes, 52, 64);
    put16(&mut bytes, 54, 56);
    put16(&mut bytes, 56, 2);
    segment(&mut bytes, 0, READ | EXECUTE, 0x1000, KERNEL_BASE, 16, 32);
    segment(
        &mut bytes,
        1,
        READ | WRITE,
        0x2000,
        KERNEL_BASE + 0x2000,
        16,
        0x1000,
    );
    bytes[0x1000..0x1010].fill(0x90);
    bytes[0x2000..0x2010].fill(0xa5);
    bytes
}

#[test]
fn valid_image_copies_exact_bytes_and_zeroes_bss_padding_and_holes() {
    let bytes = valid();
    let image = Image::parse(&bytes).unwrap();
    assert_eq!(image.entry(), KERNEL_BASE + 8);
    assert_eq!(
        (image.pages(), image.segment_count(), image.image_size()),
        (3, 2, 0x3000)
    );
    let mut destination = vec![0xcc; image.image_size()];
    image.copy_into(&mut destination).unwrap();
    assert_eq!(&destination[..16], &[0x90; 16]);
    assert!(destination[16..0x2000].iter().all(|byte| *byte == 0));
    assert_eq!(&destination[0x2000..0x2010], &[0xa5; 16]);
    assert!(destination[0x2010..].iter().all(|byte| *byte == 0));
    let mut wrong_size = [0xcc; 16];
    assert_eq!(
        image.copy_into(&mut wrong_size),
        Err(Error::DestinationSize)
    );
    assert_eq!(wrong_size, [0xcc; 16]);
}

#[test]
fn every_truncated_prefix_is_rejected_before_it_can_produce_a_load_plan() {
    let bytes = valid();
    for length in 0..bytes.len() {
        assert!(Image::parse(&bytes[..length]).is_err(), "length={length}");
    }
}

#[test]
fn byte_decoding_accepts_unaligned_source_and_program_table() {
    let bytes = valid();
    let mut padded = vec![0xa5];
    padded.extend_from_slice(&bytes);
    assert!(Image::parse(&padded[1..]).is_ok());
    let mut moved = bytes;
    moved.copy_within(64..176, 65);
    put64(&mut moved, 32, 65);
    assert!(Image::parse(&moved).is_ok());
}

#[test]
fn validates_loading_header_fields_without_interpreting_unused_section_metadata() {
    for (at, value) in [
        (0, 0),
        (4, 1),
        (5, 2),
        (6, 0),
        (7, 3),
        (8, 1),
        (16, 3),
        (18, 3),
        (20, 0),
        (48, 1),
        (52, 63),
        (54, 55),
    ] {
        let mut bytes = valid();
        bytes[at] = value;
        assert!(
            matches!(Image::parse(&bytes), Err(Error::Header)),
            "byte={at}"
        );
    }
    let mut bytes = valid();
    bytes[9..16].fill(0xaa); // ABI-reserved identification padding is ignored.
    put64(&mut bytes, 40, u64::MAX); // No section-table access during loading.
    assert!(Image::parse(&bytes).is_ok());
}

#[test]
fn program_table_overflow_overlap_and_capacity_are_rejected() {
    for offset in [0, 63, u64::MAX - 1, u64::MAX] {
        let mut bytes = valid();
        put64(&mut bytes, 32, offset);
        assert!(matches!(Image::parse(&bytes), Err(Error::ProgramHeaders)));
    }
    for count in [0, MAX_PROGRAM_HEADERS as u16 + 1, u16::MAX] {
        let mut bytes = valid();
        put16(&mut bytes, 56, count);
        assert!(matches!(Image::parse(&bytes), Err(Error::Capacity)));
    }
    let mut bytes = valid();
    bytes.resize(0x30000, 0);
    put16(&mut bytes, 56, MAX_LOAD_SEGMENTS as u16 + 1);
    for index in 0..=MAX_LOAD_SEGMENTS {
        segment(
            &mut bytes,
            index,
            READ | EXECUTE,
            0x1000,
            KERNEL_BASE + index as u64 * 0x1000,
            16,
            16,
        );
    }
    assert!(matches!(Image::parse(&bytes), Err(Error::Capacity)));
}

#[test]
fn validates_file_and_identity_mapped_destination_ranges_before_copy() {
    for (field, value, error) in [
        (8, u64::MAX, Error::FileRange),
        (32, u64::MAX, Error::FileRange),
        (40, 0, Error::MemoryRange),
        (40, 15, Error::MemoryRange),
        (40, u64::MAX, Error::MemoryRange),
        (40, MAX_IMAGE_BYTES as u64 + 1, Error::MemoryRange),
        (24, KERNEL_BASE + 4096, Error::MemoryRange),
        (16, u64::MAX - 15, Error::MemoryRange),
    ] {
        let mut bytes = valid();
        put64(&mut bytes, 64 + field, value);
        assert!(
            matches!(Image::parse(&bytes), Err(actual) if actual == error),
            "field={field} value={value}"
        );
    }
    let mut bytes = valid();
    segment(
        &mut bytes,
        0,
        READ | EXECUTE,
        0x1000,
        KERNEL_BASE - 4096,
        16,
        32,
    );
    assert!(matches!(Image::parse(&bytes), Err(Error::MemoryRange)));
}

#[test]
fn accepts_bss_without_file_bytes_but_never_enters_uninitialized_memory() {
    let mut bytes = valid();
    put64(&mut bytes, 64 + 56 + 32, 0);
    let image = Image::parse(&bytes).unwrap();
    let mut destination = vec![0xcc; image.image_size()];
    image.copy_into(&mut destination).unwrap();
    assert!(destination[0x2000..].iter().all(|byte| *byte == 0));
    for entry in [
        0,
        KERNEL_BASE + 16,
        KERNEL_BASE + 4096,
        KERNEL_BASE + 0x2000,
    ] {
        put64(&mut bytes, 24, entry);
        assert!(matches!(Image::parse(&bytes), Err(Error::Entry)));
    }
}

#[test]
fn rejects_unknown_permissions_and_writable_code() {
    for flags in [
        0,
        EXECUTE,
        WRITE,
        READ | WRITE | EXECUTE,
        READ | 8,
        u32::MAX,
    ] {
        let mut bytes = valid();
        put32(&mut bytes, 64 + 4, flags);
        assert!(
            matches!(Image::parse(&bytes), Err(Error::Permissions)),
            "flags={flags}"
        );
    }
}

#[test]
fn validates_page_and_declared_alignment() {
    for alignment in [3, 4095, 1 << 30] {
        let mut bytes = valid();
        put64(&mut bytes, 64 + 48, alignment);
        assert!(matches!(Image::parse(&bytes), Err(Error::Alignment)));
    }
    for alignment in [0, 1, 4096] {
        let mut bytes = valid();
        put64(&mut bytes, 64 + 48, alignment);
        assert!(Image::parse(&bytes).is_ok());
    }
    let mut bytes = valid();
    put64(&mut bytes, 64 + 8, 0x1001);
    assert!(matches!(Image::parse(&bytes), Err(Error::Alignment)));
}

#[test]
fn rejects_segment_byte_overlap_page_sharing_and_reverse_order() {
    for (address, offset) in [
        (KERNEL_BASE, 0x2000),
        (KERNEL_BASE + 16, 0x1010),
        (KERNEL_BASE + 64, 0x1040),
    ] {
        let mut bytes = valid();
        segment(&mut bytes, 1, READ | WRITE, offset, address, 16, 32);
        assert!(matches!(Image::parse(&bytes), Err(Error::Overlap)));
    }
    let mut bytes = valid();
    let first: [u8; 56] = bytes[64..120].try_into().unwrap();
    bytes.copy_within(120..176, 64);
    bytes[120..176].copy_from_slice(&first);
    assert!(matches!(Image::parse(&bytes), Err(Error::Overlap)));
}

#[test]
fn source_collision_checks_include_image_holes_and_overflow() {
    let bytes = valid();
    let image = Image::parse(&bytes).unwrap();
    for source in [KERNEL_BASE, KERNEL_BASE + 4096, KERNEL_BASE - 1, u64::MAX] {
        assert_eq!(
            image.validate_source_address(source),
            Err(Error::SourceCollision)
        );
    }
    assert!(image
        .validate_source_address(KERNEL_BASE - bytes.len() as u64)
        .is_ok());
    assert!(image
        .validate_source_address(KERNEL_BASE + image.image_size() as u64)
        .is_ok());
}

#[test]
fn rejects_loader_requirements_it_cannot_fulfill() {
    for kind in [2, 3, 5, 6, 7, 0x6474_e553, u32::MAX] {
        let mut bytes = valid();
        put16(&mut bytes, 56, 3);
        put32(&mut bytes, 176, kind);
        assert!(matches!(
            Image::parse(&bytes),
            Err(Error::UnsupportedSegment)
        ));
    }
}

#[test]
fn accepts_bounded_informational_notes_null_and_non_executable_stack_only() {
    for kind in [0, 4, PT_GNU_STACK] {
        let mut bytes = valid();
        put16(&mut bytes, 56, 3);
        put32(&mut bytes, 176, kind);
        if kind == PT_GNU_STACK {
            put32(&mut bytes, 180, READ | WRITE);
        }
        assert!(Image::parse(&bytes).is_ok());
    }
    let mut bytes = valid();
    put16(&mut bytes, 56, 3);
    put32(&mut bytes, 176, PT_GNU_STACK);
    put32(&mut bytes, 180, READ | WRITE | EXECUTE);
    assert!(matches!(
        Image::parse(&bytes),
        Err(Error::UnsupportedSegment)
    ));
    put32(&mut bytes, 180, READ | WRITE);
    put64(&mut bytes, 176 + 40, 4096);
    assert!(matches!(
        Image::parse(&bytes),
        Err(Error::UnsupportedSegment)
    ));
}

#[test]
fn relro_metadata_must_describe_existing_initialized_load_bytes() {
    let mut bytes = valid();
    put16(&mut bytes, 56, 3);
    segment(&mut bytes, 2, READ, 0x2000, KERNEL_BASE + 0x2000, 16, 16);
    put32(&mut bytes, 176, PT_GNU_RELRO);
    assert!(Image::parse(&bytes).is_ok());
    put64(&mut bytes, 176 + 40, 8192);
    assert!(matches!(
        Image::parse(&bytes),
        Err(Error::UnsupportedSegment)
    ));
}

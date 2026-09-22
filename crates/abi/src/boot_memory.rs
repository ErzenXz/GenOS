//! Checked boot memory contracts. No firmware pointer is dereferenced here.
//!
//! The loader supplies an accessible byte slice; the kernel's typed BootInfo
//! still requires a valid, aligned, immutable loader-owned object. Validation
//! does not turn an arbitrary address or invalid Rust enum into a safe value.

use crate::{
    BootInfo, MemoryMapInfo, MemoryRegion, MemoryRegionKind, BOOT_INFO_MAGIC, BOOT_INFO_VERSION,
    MAX_CMDLINE_LEN, MAX_MEMORY_REGIONS,
};

const PAGE_SIZE: u64 = 4096;
const EFI_MEMORY_RUNTIME: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootMemoryError {
    Header,
    CommandLine,
    DescriptorVersion,
    DescriptorStride,
    DescriptorLength,
    Capacity,
    Empty,
    Range,
    Alignment,
    Overlap,
    Unreserved,
    Framebuffer,
}

/// Parse UEFI version-1 descriptor prefixes, advancing by the returned stride.
/// Extensions and unaligned strides are read as bytes, never as Rust references.
/// Unknown memory types and runtime-attributed descriptors stay reserved.
pub fn decode_uefi_map(
    bytes: &[u8],
    stride: usize,
    version: u32,
) -> Result<MemoryMapInfo, BootMemoryError> {
    if version != 1 {
        return Err(BootMemoryError::DescriptorVersion);
    }
    if stride < 40 {
        return Err(BootMemoryError::DescriptorStride);
    }
    if !bytes.len().is_multiple_of(stride) {
        return Err(BootMemoryError::DescriptorLength);
    }
    let count = bytes.len() / stride;
    if count == 0 {
        return Err(BootMemoryError::Empty);
    }
    if count > MAX_MEMORY_REGIONS {
        return Err(BootMemoryError::Capacity);
    }
    let mut map = MemoryMapInfo::empty();
    for (index, desc) in bytes.chunks_exact(stride).enumerate() {
        let kind = u32::from_le_bytes(desc[0..4].try_into().unwrap());
        let start = u64::from_le_bytes(desc[8..16].try_into().unwrap());
        let pages = u64::from_le_bytes(desc[24..32].try_into().unwrap());
        let attributes = u64::from_le_bytes(desc[32..40].try_into().unwrap());
        let size = pages.checked_mul(PAGE_SIZE).ok_or(BootMemoryError::Range)?;
        let kind = if attributes & EFI_MEMORY_RUNTIME != 0 {
            MemoryRegionKind::Reserved
        } else {
            match kind {
                7 => MemoryRegionKind::Usable,
                1 | 2 => MemoryRegionKind::Bootloader,
                9 | 10 => MemoryRegionKind::Acpi,
                11..=13 => MemoryRegionKind::Mmio,
                // Boot-services code/data remain reserved: firmware page
                // tables and the handoff stack may still reference them.
                _ => MemoryRegionKind::Reserved,
            }
        };
        map.regions[index] = MemoryRegion { start, size, kind };
    }
    map.region_count = count as u64;
    validate_map(&map)?;
    Ok(map)
}

pub fn validate_map(map: &MemoryMapInfo) -> Result<&[MemoryRegion], BootMemoryError> {
    let count = usize::try_from(map.region_count).map_err(|_| BootMemoryError::Capacity)?;
    if count > MAX_MEMORY_REGIONS {
        return Err(BootMemoryError::Capacity);
    }
    if count == 0 {
        return Err(BootMemoryError::Empty);
    }
    let regions = &map.regions[..count];
    for (index, region) in regions.iter().enumerate() {
        let end = range_end(region.start, region.size)?;
        if !region.start.is_multiple_of(PAGE_SIZE) || !region.size.is_multiple_of(PAGE_SIZE) {
            return Err(BootMemoryError::Alignment);
        }
        for previous in &regions[..index] {
            // Previous entries have already passed checked arithmetic.
            if region.start < previous.start + previous.size && previous.start < end {
                return Err(BootMemoryError::Overlap);
            }
        }
    }
    Ok(regions)
}

fn range_end(start: u64, size: u64) -> Result<u64, BootMemoryError> {
    if size == 0 {
        return Err(BootMemoryError::Range);
    }
    start.checked_add(size).ok_or(BootMemoryError::Range)
}

/// Require complete coverage by retained loader/kernel memory, including when
/// the range spans adjacent descriptors in an unsorted firmware map.
pub fn require_retained(
    regions: &[MemoryRegion],
    start: u64,
    size: u64,
) -> Result<(), BootMemoryError> {
    let end = range_end(start, size)?;
    let mut cursor = start;
    while cursor < end {
        let region = regions
            .iter()
            .find(|region| {
                matches!(
                    region.kind,
                    MemoryRegionKind::Bootloader | MemoryRegionKind::Kernel
                ) && region.start <= cursor
                    && region
                        .start
                        .checked_add(region.size)
                        .is_some_and(|end| cursor < end)
            })
            .ok_or(BootMemoryError::Unreserved)?;
        cursor = (region.start + region.size).min(end);
    }
    Ok(())
}

/// Check fields before the kernel consumes command-line, initrd or RAM ranges.
/// Kernel image bounds come from its own linker, not from the handoff input.
pub fn validate_boot_info(
    info: &BootInfo,
    info_address: u64,
    kernel_start: u64,
    kernel_size: u64,
) -> Result<(), BootMemoryError> {
    if info.magic != BOOT_INFO_MAGIC || info.version != BOOT_INFO_VERSION {
        return Err(BootMemoryError::Header);
    }
    let len = info.cmdline_len as usize;
    if len > MAX_CMDLINE_LEN || core::str::from_utf8(&info.cmdline[..len]).is_err() {
        return Err(BootMemoryError::CommandLine);
    }
    let regions = validate_map(&info.memory_map)?;
    require_retained(
        regions,
        info_address,
        core::mem::size_of::<BootInfo>() as u64,
    )?;
    require_retained(regions, kernel_start, kernel_size)?;
    if info.initrd.size == 0 {
        if info.initrd.base != 0 {
            return Err(BootMemoryError::Range);
        }
    } else {
        require_retained(regions, info.initrd.base, info.initrd.size)?;
    }
    let fb = info.framebuffer;
    if fb.size == 0 {
        if fb.base != 0 || fb.width != 0 || fb.height != 0 || fb.stride != 0 {
            return Err(BootMemoryError::Framebuffer);
        }
    } else {
        let end = range_end(fb.base, fb.size)?;
        let needed = u64::from(fb.stride)
            .checked_mul(u64::from(fb.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(BootMemoryError::Framebuffer)?;
        if fb.width == 0
            || fb.height == 0
            || fb.stride < fb.width
            || needed > fb.size
            || regions.iter().any(|r| {
                r.kind == MemoryRegionKind::Usable && r.start < end && fb.base < r.start + r.size
            })
        {
            return Err(BootMemoryError::Framebuffer);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(kind: u32, start: u64, pages: u64, attributes: u64) -> [u8; 48] {
        let mut d = [0; 48];
        d[..4].copy_from_slice(&kind.to_le_bytes());
        d[8..16].copy_from_slice(&start.to_le_bytes());
        d[24..32].copy_from_slice(&pages.to_le_bytes());
        d[32..40].copy_from_slice(&attributes.to_le_bytes());
        d[40..].fill(0xa5);
        d
    }

    #[test]
    fn extended_and_unaligned_descriptors_use_returned_stride() {
        let a = descriptor(7, 0x8000, 2, 0);
        let b = descriptor(2, 0x1000, 3, 0);
        for stride in [40, 41, 48] {
            let mut raw = [0; 96];
            raw[..stride].copy_from_slice(&a[..stride]);
            raw[stride..2 * stride].copy_from_slice(&b[..stride]);
            let map = decode_uefi_map(&raw[..2 * stride], stride, 1).unwrap();
            assert_eq!(map.region_count, 2);
            assert_eq!(map.regions[0].size, 8192);
            assert_eq!(map.regions[1].kind, MemoryRegionKind::Bootloader);
        }
    }

    #[test]
    fn rejects_bad_metadata_before_reading() {
        let raw = descriptor(7, 4096, 1, 0);
        assert_eq!(
            decode_uefi_map(&raw, 0, 1),
            Err(BootMemoryError::DescriptorStride)
        );
        assert_eq!(
            decode_uefi_map(&raw, 39, 1),
            Err(BootMemoryError::DescriptorStride)
        );
        assert_eq!(
            decode_uefi_map(&raw, 48, 2),
            Err(BootMemoryError::DescriptorVersion)
        );
        assert_eq!(
            decode_uefi_map(&raw[..47], 48, 1),
            Err(BootMemoryError::DescriptorLength)
        );
        assert_eq!(decode_uefi_map(&[], 48, 1), Err(BootMemoryError::Empty));
    }

    #[test]
    fn rejects_capacity_instead_of_truncating() {
        let mut raw = [0; 48 * (MAX_MEMORY_REGIONS + 1)];
        for (i, d) in raw.chunks_exact_mut(48).enumerate() {
            d.copy_from_slice(&descriptor(7, i as u64 * 4096, 1, 0));
        }
        assert_eq!(decode_uefi_map(&raw, 48, 1), Err(BootMemoryError::Capacity));
        assert_eq!(
            decode_uefi_map(&raw[..48 * MAX_MEMORY_REGIONS], 48, 1)
                .unwrap()
                .region_count,
            MAX_MEMORY_REGIONS as u64
        );
    }

    #[test]
    fn rejects_overflow_empty_unaligned_and_overlapping_ranges() {
        for d in [
            descriptor(7, 4096, u64::MAX, 0),
            descriptor(7, u64::MAX - 4095, 1, 0),
            descriptor(7, 4096, 0, 0),
        ] {
            assert_eq!(decode_uefi_map(&d, 48, 1), Err(BootMemoryError::Range));
        }
        assert_eq!(
            decode_uefi_map(&descriptor(7, 4097, 1, 0), 48, 1),
            Err(BootMemoryError::Alignment)
        );
        let mut raw = [0; 96];
        raw[..48].copy_from_slice(&descriptor(7, 4096, 2, 0));
        raw[48..].copy_from_slice(&descriptor(0, 8192, 1, 0));
        assert_eq!(decode_uefi_map(&raw, 48, 1), Err(BootMemoryError::Overlap));
    }

    #[test]
    fn firmware_runtime_and_unknown_memory_never_become_usable() {
        for (kind, attrs) in [
            (7, EFI_MEMORY_RUNTIME),
            (0xffff, 0),
            (3, 0),
            (4, 0),
            (5, 0),
            (6, 0),
            (15, 0),
        ] {
            assert_eq!(
                decode_uefi_map(&descriptor(kind, 4096, 1, attrs), 48, 1)
                    .unwrap()
                    .regions[0]
                    .kind,
                MemoryRegionKind::Reserved
            );
        }
    }

    fn valid_info() -> BootInfo {
        let mut info = BootInfo::empty();
        info.memory_map.region_count = 3;
        info.memory_map.regions[0] = MemoryRegion {
            start: 0x1000,
            size: 0x10000,
            kind: MemoryRegionKind::Bootloader,
        };
        info.memory_map.regions[1] = MemoryRegion {
            start: 0x20000,
            size: 0x10000,
            kind: MemoryRegionKind::Kernel,
        };
        info.memory_map.regions[2] = MemoryRegion {
            start: 0x30000,
            size: 0x10000,
            kind: MemoryRegionKind::Usable,
        };
        info.initrd.base = 0x9000;
        info.initrd.size = 16;
        info.set_cmdline("console=serial");
        info
    }

    fn validate(info: &BootInfo) -> Result<(), BootMemoryError> {
        validate_boot_info(info, 0x1000, 0x20000, 0x10000)
    }

    #[test]
    fn handoff_requires_retained_kernel_info_and_initrd() {
        let mut info = valid_info();
        assert_eq!(validate(&info), Ok(()));
        for index in 0..2 {
            let old = info.memory_map.regions[index].kind;
            info.memory_map.regions[index].kind = MemoryRegionKind::Usable;
            assert_eq!(validate(&info), Err(BootMemoryError::Unreserved));
            info.memory_map.regions[index].kind = old;
        }
        info.initrd.base = 0x30000;
        assert_eq!(validate(&info), Err(BootMemoryError::Unreserved));
        info.initrd.base = u64::MAX;
        assert_eq!(validate(&info), Err(BootMemoryError::Range));
    }

    #[test]
    fn retained_coverage_allows_adjacent_unsorted_regions_but_not_holes() {
        let mut regions = [
            MemoryRegion {
                start: 0x2000,
                size: 0x1000,
                kind: MemoryRegionKind::Bootloader,
            },
            MemoryRegion {
                start: 0x1000,
                size: 0x1000,
                kind: MemoryRegionKind::Kernel,
            },
        ];
        assert_eq!(require_retained(&regions, 0x1800, 0x1800), Ok(()));
        regions[0].start = 0x3000;
        assert_eq!(
            require_retained(&regions, 0x1800, 0x1800),
            Err(BootMemoryError::Unreserved)
        );
    }

    #[test]
    fn rejects_invalid_handoff_fields_before_consumption() {
        let mut info = valid_info();
        info.cmdline_len = 129;
        assert_eq!(validate(&info), Err(BootMemoryError::CommandLine));
        info.cmdline_len = 1;
        info.cmdline[0] = 0xff;
        assert_eq!(validate(&info), Err(BootMemoryError::CommandLine));
        info.set_cmdline("ok");
        info.memory_map.region_count = 257;
        assert_eq!(validate(&info), Err(BootMemoryError::Capacity));
        info.memory_map.region_count = 3;
        info.version += 1;
        assert_eq!(validate(&info), Err(BootMemoryError::Header));
    }

    #[test]
    fn rejects_framebuffer_overflow_size_and_usable_alias() {
        let mut info = valid_info();
        info.framebuffer = crate::FramebufferInfo {
            base: 0x100000,
            size: 4096,
            width: 8,
            height: 8,
            stride: 8,
            pixel_format: crate::PixelFormat::Bgr,
        };
        assert_eq!(validate(&info), Ok(()));
        info.framebuffer.base = 0x30000;
        assert_eq!(validate(&info), Err(BootMemoryError::Framebuffer));
        info.framebuffer.base = u64::MAX;
        assert_eq!(validate(&info), Err(BootMemoryError::Range));
        info.framebuffer.base = 0x100000;
        info.framebuffer.size = 1;
        assert_eq!(validate(&info), Err(BootMemoryError::Framebuffer));
    }
}

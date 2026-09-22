//! A bounded load plan for GenOS's fixed-address, statically linked kernel.
//!
//! All loading metadata is validated before allocation or destination writes.
//! ELF structures are decoded from bytes, never borrowed at assumed alignment.

const HEADER_BYTES: usize = 64;
const PROGRAM_HEADER_BYTES: usize = 56;
pub const PAGE_BYTES: usize = 4096;
pub const KERNEL_BASE: u64 = 32 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PROGRAM_HEADERS: usize = 32;
pub const MAX_LOAD_SEGMENTS: usize = 16;
const PT_LOAD: u32 = 1;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const READ: u32 = 4;
const WRITE: u32 = 2;
const EXECUTE: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Header,
    ProgramHeaders,
    UnsupportedSegment,
    FileRange,
    MemoryRange,
    Alignment,
    Permissions,
    Overlap,
    Entry,
    Capacity,
    SourceCollision,
    DestinationSize,
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    address: u64,
    memory_size: usize,
    file_start: usize,
    file_end: usize,
    flags: u32,
}

impl Segment {
    const EMPTY: Self = Self {
        address: 0,
        memory_size: 0,
        file_start: 0,
        file_end: 0,
        flags: 0,
    };
    fn memory_end(self) -> u64 {
        self.address + self.memory_size as u64
    }
    fn page_end(self) -> u64 {
        (self.memory_end() + PAGE_BYTES as u64 - 1) & !(PAGE_BYTES as u64 - 1)
    }
}

/// An immutable source borrow binds every validated file range to its bytes.
/// Private fields prevent callers from constructing an unchecked load plan.
#[derive(Debug)]
pub struct Image<'a> {
    bytes: &'a [u8],
    segments: [Segment; MAX_LOAD_SEGMENTS],
    count: usize,
    entry: u64,
    image_size: usize,
}

impl<'a> Image<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_BYTES
            || &bytes[..4] != b"\x7fELF"
            || bytes[4..9] != [2, 1, 1, 0, 0]
            || u16_at(bytes, 16) != 2
            || u16_at(bytes, 18) != 62
            || u32_at(bytes, 20) != 1
            || u32_at(bytes, 48) != 0
            || usize::from(u16_at(bytes, 52)) != HEADER_BYTES
            || usize::from(u16_at(bytes, 54)) != PROGRAM_HEADER_BYTES
        {
            return Err(Error::Header);
        }
        let count = usize::from(u16_at(bytes, 56));
        if count == 0 || count > MAX_PROGRAM_HEADERS {
            return Err(Error::Capacity);
        }
        let table_start = usize::try_from(u64_at(bytes, 32)).map_err(|_| Error::ProgramHeaders)?;
        let table_end = table_start
            .checked_add(count * PROGRAM_HEADER_BYTES)
            .ok_or(Error::ProgramHeaders)?;
        if table_start < HEADER_BYTES || table_end > bytes.len() {
            return Err(Error::ProgramHeaders);
        }
        let mut image = Self {
            bytes,
            segments: [Segment::EMPTY; MAX_LOAD_SEGMENTS],
            count: 0,
            entry: u64_at(bytes, 24),
            image_size: 0,
        };
        let mut stack_seen = false;
        let mut relro = None;
        for header in bytes[table_start..table_end].chunks_exact(PROGRAM_HEADER_BYTES) {
            match u32_at(header, 0) {
                0 => {} // PT_NULL carries no defined loading requirements.
                PT_LOAD => {
                    if image.count == MAX_LOAD_SEGMENTS {
                        return Err(Error::Capacity);
                    }
                    let segment = load_segment(bytes, header)?;
                    if image.count != 0
                        && segment.address < image.segments[image.count - 1].page_end()
                    {
                        return Err(Error::Overlap);
                    }
                    image.segments[image.count] = segment;
                    image.count += 1;
                }
                4 => {
                    file_range(bytes, header)?;
                } // Informational PT_NOTE only.
                PT_GNU_STACK => {
                    if stack_seen
                        || u32_at(header, 4) != READ | WRITE
                        || header[8..56].iter().any(|byte| *byte != 0)
                    {
                        return Err(Error::UnsupportedSegment);
                    }
                    stack_seen = true;
                }
                PT_GNU_RELRO => {
                    if relro.is_some() {
                        return Err(Error::UnsupportedSegment);
                    }
                    relro = Some(load_segment(bytes, header)?);
                }
                // Dynamic linking, interpreters, TLS, PHDR mapping and unknown
                // OS/processor extensions have no GenOS boot-loader contract.
                _ => return Err(Error::UnsupportedSegment),
            }
        }
        if image.count == 0 || image.segments[0].address != KERNEL_BASE {
            return Err(Error::MemoryRange);
        }
        let segments = &image.segments[..image.count];
        if !segments.iter().any(|segment| {
            segment.flags & EXECUTE != 0
                && image.entry >= segment.address
                && image.entry < segment.address + (segment.file_end - segment.file_start) as u64
        }) {
            return Err(Error::Entry);
        }
        if let Some(relro) = relro {
            if relro.flags != READ
                || !segments.iter().any(|segment| {
                    relro.address >= segment.address
                        && relro.memory_end() <= segment.memory_end()
                        && relro.file_start >= segment.file_start
                        && relro.file_end <= segment.file_end
                        && relro.address - segment.address
                            == (relro.file_start - segment.file_start) as u64
                })
            {
                return Err(Error::UnsupportedSegment);
            }
        }
        image.image_size = (segments.last().unwrap().page_end() - KERNEL_BASE) as usize;
        Ok(image)
    }

    pub fn entry(&self) -> u64 {
        self.entry
    }
    pub fn image_start(&self) -> u64 {
        KERNEL_BASE
    }
    pub fn image_size(&self) -> usize {
        self.image_size
    }
    pub fn segment_count(&self) -> usize {
        self.count
    }
    pub fn pages(&self) -> usize {
        self.image_size / PAGE_BYTES
    }

    /// The firmware adapter supplies the live source allocation's actual base.
    /// Refuse collision with the entire grant, including zeroed padding/holes.
    pub fn validate_source_address(&self, address: u64) -> Result<(), Error> {
        let end = address
            .checked_add(self.bytes.len() as u64)
            .ok_or(Error::SourceCollision)?;
        if address < KERNEL_BASE + self.image_size as u64 && KERNEL_BASE < end {
            return Err(Error::SourceCollision);
        }
        Ok(())
    }

    /// Safe host-testable copy operation. No fallible operation follows the
    /// first write: all source and destination bounds belong to the load plan.
    pub fn copy_into(&self, destination: &mut [u8]) -> Result<(), Error> {
        if destination.len() != self.image_size {
            return Err(Error::DestinationSize);
        }
        destination.fill(0);
        for (start, file) in self.copies() {
            destination[start..start + file.len()].copy_from_slice(file);
        }
        Ok(())
    }

    /// Validated file slices and offsets within the image allocation.
    pub fn copies(&self) -> impl Iterator<Item = (usize, &[u8])> {
        self.segments[..self.count].iter().map(|segment| {
            (
                (segment.address - KERNEL_BASE) as usize,
                &self.bytes[segment.file_start..segment.file_end],
            )
        })
    }
}

fn load_segment(bytes: &[u8], header: &[u8]) -> Result<Segment, Error> {
    let flags = u32_at(header, 4);
    if flags & READ == 0
        || flags & !(READ | WRITE | EXECUTE) != 0
        || flags & (WRITE | EXECUTE) == WRITE | EXECUTE
    {
        return Err(Error::Permissions);
    }
    let (file_start, file_end) = file_range(bytes, header)?;
    let address = u64_at(header, 16);
    let physical = u64_at(header, 24);
    let size = u64_at(header, 40);
    let end = address.checked_add(size).ok_or(Error::MemoryRange)?;
    if address != physical
        || address < KERNEL_BASE
        || size == 0
        || end > KERNEL_BASE + MAX_IMAGE_BYTES as u64
        || size < (file_end - file_start) as u64
    {
        return Err(Error::MemoryRange);
    }
    let alignment = u64_at(header, 48);
    if (alignment > 1
        && (!alignment.is_power_of_two() || address % alignment != file_start as u64 % alignment))
        || address % PAGE_BYTES as u64 != file_start as u64 % PAGE_BYTES as u64
    {
        return Err(Error::Alignment);
    }
    Ok(Segment {
        address,
        memory_size: size as usize,
        file_start,
        file_end,
        flags,
    })
}

fn file_range(bytes: &[u8], header: &[u8]) -> Result<(usize, usize), Error> {
    let start = u64_at(header, 8);
    let end = start
        .checked_add(u64_at(header, 32))
        .ok_or(Error::FileRange)?;
    let start = usize::try_from(start).map_err(|_| Error::FileRange)?;
    let end = usize::try_from(end).map_err(|_| Error::FileRange)?;
    if end > bytes.len() {
        return Err(Error::FileRange);
    }
    Ok((start, end))
}

// Callers use fixed offsets only after validating a complete header prefix.
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
#[path = "kernel_elf_tests.rs"]
mod tests;

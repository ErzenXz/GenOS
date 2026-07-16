const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const PT_LOAD: u32 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const MAX_PROGRAM_HEADERS: usize = 16;

pub const FLAG_EXECUTE: u32 = 1;
pub const FLAG_WRITE: u32 = 2;
pub const FLAG_READ: u32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfError {
    Truncated,
    InvalidMagic,
    Unsupported,
    InvalidHeaders,
    InvalidSegment,
}

#[derive(Clone, Copy)]
pub struct Segment<'a> {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub memory_size: u64,
    pub flags: u32,
    pub align: u64,
    pub file_data: &'a [u8],
}

pub struct ElfImage<'a> {
    bytes: &'a [u8],
    entry: u64,
    program_offset: usize,
    program_count: usize,
}

impl<'a> ElfImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
        if bytes.len() < ELF_HEADER_SIZE {
            return Err(ElfError::Truncated);
        }
        if bytes.get(0..4) != Some(b"\x7fELF") {
            return Err(ElfError::InvalidMagic);
        }
        if bytes[4] != 2
            || bytes[5] != 1
            || bytes[6] != 1
            || read_u16(bytes, 16)? != ET_EXEC
            || read_u16(bytes, 18)? != EM_X86_64
            || read_u32(bytes, 20)? != 1
        {
            return Err(ElfError::Unsupported);
        }
        if read_u16(bytes, 52)? as usize != ELF_HEADER_SIZE
            || read_u16(bytes, 54)? as usize != PROGRAM_HEADER_SIZE
        {
            return Err(ElfError::InvalidHeaders);
        }

        let entry = read_u64(bytes, 24)?;
        let program_offset =
            usize::try_from(read_u64(bytes, 32)?).map_err(|_| ElfError::InvalidHeaders)?;
        let program_count = read_u16(bytes, 56)? as usize;
        if entry == 0
            || program_offset < ELF_HEADER_SIZE
            || program_count == 0
            || program_count > MAX_PROGRAM_HEADERS
        {
            return Err(ElfError::InvalidHeaders);
        }
        let table_size = program_count
            .checked_mul(PROGRAM_HEADER_SIZE)
            .ok_or(ElfError::InvalidHeaders)?;
        let table_end = program_offset
            .checked_add(table_size)
            .ok_or(ElfError::InvalidHeaders)?;
        if table_end > bytes.len() {
            return Err(ElfError::Truncated);
        }

        let image = Self {
            bytes,
            entry,
            program_offset,
            program_count,
        };
        let mut load_segments = 0usize;
        for segment in image.segments() {
            segment?;
            load_segments += 1;
        }
        if load_segments == 0 {
            return Err(ElfError::InvalidSegment);
        }
        Ok(image)
    }

    pub const fn entry(&self) -> u64 {
        self.entry
    }

    pub const fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    pub fn segments(&self) -> SegmentIter<'a> {
        SegmentIter {
            bytes: self.bytes,
            next: 0,
            program_offset: self.program_offset,
            program_count: self.program_count,
        }
    }
}

pub struct SegmentIter<'a> {
    bytes: &'a [u8],
    next: usize,
    program_offset: usize,
    program_count: usize,
}

impl<'a> Iterator for SegmentIter<'a> {
    type Item = Result<Segment<'a>, ElfError>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.program_count {
            let offset = self.program_offset + self.next * PROGRAM_HEADER_SIZE;
            self.next += 1;
            let kind = match read_u32(self.bytes, offset) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            if kind != PT_LOAD {
                continue;
            }
            return Some(read_segment(self.bytes, offset));
        }
        None
    }
}

fn read_segment(bytes: &[u8], offset: usize) -> Result<Segment<'_>, ElfError> {
    let flags = read_u32(bytes, offset + 4)?;
    let raw_file_offset = read_u64(bytes, offset + 8)?;
    let file_offset = usize::try_from(raw_file_offset).map_err(|_| ElfError::InvalidSegment)?;
    let virtual_address = read_u64(bytes, offset + 16)?;
    let file_size =
        usize::try_from(read_u64(bytes, offset + 32)?).map_err(|_| ElfError::InvalidSegment)?;
    let memory_size = read_u64(bytes, offset + 40)?;
    let align = read_u64(bytes, offset + 48)?;
    if memory_size == 0 || memory_size < file_size as u64 || align == 0 || !align.is_power_of_two()
    {
        return Err(ElfError::InvalidSegment);
    }
    let file_end = file_offset
        .checked_add(file_size)
        .ok_or(ElfError::InvalidSegment)?;
    let file_data = bytes
        .get(file_offset..file_end)
        .ok_or(ElfError::Truncated)?;
    Ok(Segment {
        file_offset: raw_file_offset,
        virtual_address,
        memory_size,
        flags,
        align,
        file_data,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ElfError> {
    let value = bytes.get(offset..offset + 2).ok_or(ElfError::Truncated)?;
    Ok(u16::from_le_bytes(
        value.try_into().map_err(|_| ElfError::Truncated)?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ElfError> {
    let value = bytes.get(offset..offset + 4).ok_or(ElfError::Truncated)?;
    Ok(u32::from_le_bytes(
        value.try_into().map_err(|_| ElfError::Truncated)?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ElfError> {
    let value = bytes.get(offset..offset + 8).ok_or(ElfError::Truncated)?;
    Ok(u64::from_le_bytes(
        value.try_into().map_err(|_| ElfError::Truncated)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_elf() -> [u8; 320] {
        let mut bytes = [0u8; 320];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        put_u16(&mut bytes, 16, ET_EXEC);
        put_u16(&mut bytes, 18, EM_X86_64);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 24, 0x4000);
        put_u64(&mut bytes, 32, ELF_HEADER_SIZE as u64);
        put_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16);
        put_u16(&mut bytes, 54, PROGRAM_HEADER_SIZE as u16);
        put_u16(&mut bytes, 56, 1);
        put_u32(&mut bytes, 64, PT_LOAD);
        put_u32(&mut bytes, 68, FLAG_READ | FLAG_EXECUTE);
        put_u64(&mut bytes, 72, 256);
        put_u64(&mut bytes, 80, 0x4000);
        put_u64(&mut bytes, 96, 8);
        put_u64(&mut bytes, 104, 16);
        put_u64(&mut bytes, 112, 256);
        bytes[256..264].copy_from_slice(b"GENOSELF");
        bytes
    }

    #[test]
    fn parses_bounded_elf64_load_segments() {
        let bytes = valid_elf();
        let image = ElfImage::parse(&bytes).unwrap();
        let segment = image.segments().next().unwrap().unwrap();
        assert_eq!(image.entry(), 0x4000);
        assert_eq!(segment.file_offset, 256);
        assert_eq!(segment.virtual_address, 0x4000);
        assert_eq!(segment.memory_size, 16);
        assert_eq!(segment.file_data, b"GENOSELF");
        assert_eq!(segment.flags, FLAG_READ | FLAG_EXECUTE);
    }

    #[test]
    fn rejects_truncated_program_tables() {
        let mut bytes = valid_elf();
        put_u16(&mut bytes, 56, 8);
        assert_eq!(ElfImage::parse(&bytes).err(), Some(ElfError::Truncated));
    }

    #[test]
    fn rejects_segments_larger_on_disk_than_in_memory() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, 96, 32);
        put_u64(&mut bytes, 104, 16);
        assert_eq!(
            ElfImage::parse(&bytes).err(),
            Some(ElfError::InvalidSegment)
        );
    }

    #[test]
    fn rejects_program_table_inside_elf_header() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, 32, 32);
        assert_eq!(
            ElfImage::parse(&bytes).err(),
            Some(ElfError::InvalidHeaders)
        );
    }

    #[test]
    fn rejects_non_power_of_two_segment_alignment() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, 112, 24);
        assert_eq!(
            ElfImage::parse(&bytes).err(),
            Some(ElfError::InvalidSegment)
        );
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}

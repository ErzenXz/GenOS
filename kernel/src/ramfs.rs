use core::{slice, str};

const MAGIC: &[u8; 4] = b"GRD1";

#[derive(Clone, Copy)]
pub struct File<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

pub struct RamFs<'a> {
    bytes: &'a [u8],
    count: usize,
}

impl<'a> RamFs<'a> {
    pub fn from_initrd(base: u64, size: u64) -> Self {
        if base == 0 || size < 8 {
            return Self {
                bytes: &[],
                count: 0,
            };
        }
        let bytes = unsafe { slice::from_raw_parts(base as *const u8, size as usize) };
        if bytes.get(0..4) != Some(MAGIC) {
            return Self {
                bytes: &[],
                count: 0,
            };
        }
        let count = read_u32(bytes, 4).unwrap_or(0) as usize;
        Self { bytes, count }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn iter(&self) -> RamFsIter<'a> {
        RamFsIter {
            bytes: self.bytes,
            remaining: self.count,
            offset: 8,
        }
    }
}

pub struct RamFsIter<'a> {
    bytes: &'a [u8],
    remaining: usize,
    offset: usize,
}

impl<'a> Iterator for RamFsIter<'a> {
    type Item = File<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let name_len = read_u16(self.bytes, self.offset)? as usize;
        let data_len = read_u32(self.bytes, self.offset + 2)? as usize;
        let name_start = self.offset + 6;
        let name_end = name_start.checked_add(name_len)?;
        let data_start = name_end;
        let data_end = data_start.checked_add(data_len)?;
        let name = str::from_utf8(self.bytes.get(name_start..name_end)?).ok()?;
        let data = self.bytes.get(data_start..data_end)?;
        self.offset = data_end;
        self.remaining -= 1;
        Some(File { name, data })
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

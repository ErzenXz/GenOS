//! Whole-image admission for the current fixed-address native application ABI.
//!
//! The immutable plan borrows the exact validated source. Parsing cannot allocate
//! frames, create owners, copy to physical memory or publish process authority.
//! The paging adapter consumes this plan only after every segment has passed.

use crate::elf::{ElfImage, FLAG_EXECUTE, FLAG_READ, FLAG_WRITE};

pub const PAGE_SIZE: u64 = 4096;
pub const USER_BASE: u64 = 0x0000_4000_0000_0000;
pub const USER_CODE: u64 = USER_BASE + 0x1000;
pub const USER_DATA: u64 = USER_BASE + 0x2000;
pub const USER_STACK_GUARD: u64 = USER_BASE + 0xb000;
pub const USER_STACK_BOTTOM: u64 = USER_BASE + 0xc000;
pub const USER_STACK_PAGES: usize = 4;
pub const USER_STACK_TOP: u64 = USER_STACK_BOTTOM + USER_STACK_PAGES as u64 * PAGE_SIZE;
pub const IMAGE_PAGE_CAPACITY: usize = ((USER_STACK_GUARD - USER_CODE) / PAGE_SIZE) as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidElf,
    InvalidLayout,
}

#[derive(Clone, Copy, Debug)]
pub struct Page<'a> {
    address: u64,
    bytes: &'a [u8],
    writable: bool,
    executable: bool,
    process_data: bool,
}

impl<'a> Page<'a> {
    const EMPTY: Self = Self {
        address: 0,
        bytes: &[],
        writable: false,
        executable: false,
        process_data: false,
    };

    pub const fn address(self) -> u64 {
        self.address
    }
    pub const fn file_bytes(self) -> &'a [u8] {
        self.bytes
    }
    pub const fn writable(self) -> bool {
        self.writable
    }
    pub const fn executable(self) -> bool {
        self.executable
    }
    pub const fn process_data(self) -> bool {
        self.process_data
    }
}

#[derive(Debug)]
pub struct Plan<'a> {
    pages: [Page<'a>; IMAGE_PAGE_CAPACITY],
    entry: u64,
    executable_start: u64,
    executable_end: u64,
    segment_count: u8,
    page_count: u8,
}

impl<'a> Plan<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let image = ElfImage::parse(bytes).map_err(|_| Error::InvalidElf)?;
        if image.entry() < USER_CODE || image.entry() >= USER_STACK_GUARD {
            return Err(Error::InvalidLayout);
        }
        let mut plan = Self {
            pages: [Page::EMPTY; IMAGE_PAGE_CAPACITY],
            entry: image.entry(),
            executable_start: u64::MAX,
            executable_end: 0,
            segment_count: 0,
            page_count: 0,
        };
        let mut mapped_pages = 0u64;
        let mut executable_pages = 0u64;
        let mut entry_is_executable = false;
        let mut process_data = false;
        for segment in image.segments() {
            let segment = segment.map_err(|_| Error::InvalidElf)?;
            let writable = segment.flags & FLAG_WRITE != 0;
            let executable = segment.flags & FLAG_EXECUTE != 0;
            if segment.flags & FLAG_READ == 0
                || segment.flags & !(FLAG_READ | FLAG_WRITE | FLAG_EXECUTE) != 0
                || (writable && executable)
                || segment.align < PAGE_SIZE
                || segment.virtual_address & (segment.align - 1)
                    != segment.file_offset & (segment.align - 1)
                || !segment.virtual_address.is_multiple_of(PAGE_SIZE)
            {
                return Err(Error::InvalidLayout);
            }
            let end = segment
                .virtual_address
                .checked_add(segment.memory_size)
                .ok_or(Error::InvalidLayout)?;
            if segment.virtual_address < USER_CODE || end > USER_STACK_GUARD {
                return Err(Error::InvalidLayout);
            }
            if executable {
                entry_is_executable |= plan.entry >= segment.virtual_address && plan.entry < end;
                plan.executable_start = plan.executable_start.min(segment.virtual_address);
                plan.executable_end = plan.executable_end.max(end);
            }
            let pages = segment.memory_size.div_ceil(PAGE_SIZE);
            if pages == 0 || pages > 16 {
                return Err(Error::InvalidLayout);
            }
            if executable {
                executable_pages += pages;
                if executable_pages > genos_abi::USER_EXECUTABLE_PAGE_CAPACITY {
                    return Err(Error::InvalidLayout);
                }
            }
            for index in 0..pages {
                let address = segment.virtual_address + index * PAGE_SIZE;
                let image_page = (address - USER_CODE) / PAGE_SIZE;
                if image_page >= IMAGE_PAGE_CAPACITY as u64 || mapped_pages & (1 << image_page) != 0
                {
                    return Err(Error::InvalidLayout);
                }
                mapped_pages |= 1 << image_page;
                let offset = (index * PAGE_SIZE) as usize;
                let copy_len = segment
                    .file_data
                    .len()
                    .saturating_sub(offset)
                    .min(PAGE_SIZE as usize);
                let bytes = if copy_len == 0 {
                    &segment.file_data[..0]
                } else {
                    &segment.file_data[offset..offset + copy_len]
                };
                let is_process_data = address == USER_DATA
                    && writable
                    && segment.memory_size >= 16
                    && segment.file_data.len() >= 16;
                process_data |= is_process_data;
                plan.pages[usize::from(plan.page_count)] = Page {
                    address,
                    bytes,
                    writable,
                    executable,
                    process_data: is_process_data,
                };
                plan.page_count += 1;
            }
            plan.segment_count += 1;
        }
        if !entry_is_executable || !process_data {
            return Err(Error::InvalidLayout);
        }
        Ok(plan)
    }

    pub fn pages(&self) -> impl ExactSizeIterator<Item = Page<'a>> + '_ {
        self.pages[..usize::from(self.page_count)].iter().copied()
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn executable_start(&self) -> u64 {
        self.executable_start
    }
    pub const fn executable_end(&self) -> u64 {
        self.executable_end
    }
    pub const fn segment_count(&self) -> u8 {
        self.segment_count
    }
    pub const fn page_count(&self) -> u8 {
        self.page_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn image() -> std::vec::Vec<u8> {
        let mut bytes = std::vec![0; 8192 + 16];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        put(&mut bytes, 24, USER_CODE);
        put(&mut bytes, 32, 64);
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&2u16.to_le_bytes());
        for (header, offset, address, flags) in
            [(64, 4096, USER_CODE, 5u32), (120, 8192, USER_DATA, 6)]
        {
            bytes[header..header + 4].copy_from_slice(&1u32.to_le_bytes());
            bytes[header + 4..header + 8].copy_from_slice(&flags.to_le_bytes());
            put(&mut bytes, header + 8, offset);
            put(&mut bytes, header + 16, address);
            put(&mut bytes, header + 32, 16);
            put(&mut bytes, header + 40, PAGE_SIZE);
            put(&mut bytes, header + 48, PAGE_SIZE);
        }
        bytes[4096..4112].fill(0xc3);
        bytes[8192..].copy_from_slice(b"GENOS-NATIVE-ELF");
        bytes
    }

    #[test]
    fn whole_plan_preserves_source_pages_permissions_entry_and_counts() {
        let bytes = image();
        let plan = Plan::parse(&bytes).unwrap();
        assert_eq!(plan.entry(), USER_CODE);
        assert_eq!(plan.executable_start(), USER_CODE);
        assert_eq!(plan.executable_end(), USER_CODE + PAGE_SIZE);
        assert_eq!((plan.segment_count(), plan.page_count()), (2, 2));
        let pages: std::vec::Vec<_> = plan.pages().collect();
        assert_eq!(pages[0].address(), USER_CODE);
        assert_eq!(pages[0].file_bytes(), &bytes[4096..4112]);
        assert!(pages[0].executable() && !pages[0].writable() && !pages[0].process_data());
        assert_eq!(pages[1].address(), USER_DATA);
        assert_eq!(pages[1].file_bytes(), &bytes[8192..]);
        assert!(!pages[1].executable() && pages[1].writable() && pages[1].process_data());
    }

    #[test]
    fn every_truncation_is_rejected_without_changing_source() {
        let bytes = image();
        let before = bytes.clone();
        for end in 0..bytes.len() {
            assert!(
                Plan::parse(&bytes[..end]).is_err(),
                "accepted truncation {end}"
            );
        }
        assert_eq!(bytes, before);
    }

    #[test]
    fn late_permissions_alignment_overlap_and_overflow_reject_the_entire_image() {
        for (offset, value) in [
            (124, 7),
            (124, 2),
            (124, 0x86),
            (136, USER_CODE),
            (136, USER_DATA + 1),
            (136, u64::MAX - 4095),
            (160, u64::MAX),
            (168, 1),
            (168, 1 << 47),
            (128, 8193),
        ] {
            let mut bytes = image();
            if offset == 124 {
                bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
            } else {
                put(&mut bytes, offset, value);
            }
            assert!(
                Plan::parse(&bytes).is_err(),
                "accepted late invalid field {offset}={value:x}"
            );
        }
    }

    #[test]
    fn entry_must_be_inside_an_executable_segment_and_not_data_or_page_padding() {
        for (entry, accepted) in [
            (USER_CODE, true),
            (USER_CODE + PAGE_SIZE - 1, true),
            (USER_DATA, false),
            (USER_CODE - 1, false),
            (USER_STACK_GUARD, false),
            (u64::MAX, false),
        ] {
            let mut bytes = image();
            put(&mut bytes, 24, entry);
            assert_eq!(Plan::parse(&bytes).is_ok(), accepted);
        }
        let mut bytes = image();
        put(&mut bytes, 104, 16);
        put(&mut bytes, 24, USER_CODE + 16);
        assert_eq!(Plan::parse(&bytes).err(), Some(Error::InvalidLayout));
    }

    #[test]
    fn data_mapping_must_be_writable_at_the_abi_address_and_have_initial_bytes() {
        for (offset, value) in [(124, 4), (136, USER_DATA + PAGE_SIZE), (152, 15), (160, 15)] {
            let mut bytes = image();
            if offset == 124 {
                bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
            } else {
                put(&mut bytes, offset, value);
            }
            assert!(Plan::parse(&bytes).is_err());
        }
    }

    #[test]
    fn eight_page_shell_budget_is_accepted_and_ninth_executable_page_is_denied() {
        let mut bytes = image();
        put(&mut bytes, 24, USER_DATA + PAGE_SIZE);
        put(&mut bytes, 80, USER_DATA + PAGE_SIZE);
        put(&mut bytes, 104, 8 * PAGE_SIZE);
        let plan = Plan::parse(&bytes).unwrap();
        assert_eq!((plan.segment_count(), plan.page_count()), (2, 9));
        assert_eq!(plan.pages().filter(|page| page.executable()).count(), 8);
        assert_eq!(
            plan.pages()
                .filter(|page| page.file_bytes().is_empty())
                .count(),
            7
        );
        put(&mut bytes, 104, 9 * PAGE_SIZE);
        assert_eq!(Plan::parse(&bytes).err(), Some(Error::InvalidLayout));
    }

    #[test]
    fn partial_file_tail_and_bss_have_exact_bounded_copy_slices() {
        let mut bytes = image();
        // Move data to a later file offset so the executable can span two pages.
        bytes.resize(4 * PAGE_SIZE as usize + 16, 0x5a);
        put(&mut bytes, 128, 4 * PAGE_SIZE);
        put(&mut bytes, 80, USER_DATA + PAGE_SIZE);
        put(&mut bytes, 24, USER_DATA + PAGE_SIZE);
        put(&mut bytes, 96, PAGE_SIZE + 7);
        put(&mut bytes, 104, 3 * PAGE_SIZE);
        let plan = Plan::parse(&bytes).unwrap();
        let pages: std::vec::Vec<_> = plan.pages().collect();
        assert_eq!(
            pages
                .iter()
                .map(|page| page.file_bytes().len())
                .collect::<std::vec::Vec<_>>(),
            [4096, 7, 0, 16]
        );
        assert_eq!(pages[1].file_bytes(), &bytes[8192..8199]);
    }

    #[test]
    fn nonload_headers_cannot_supply_entry_or_required_data_authority() {
        let mut bytes = image();
        bytes[120..124].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Plan::parse(&bytes).err(), Some(Error::InvalidLayout));
        bytes[64..68].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Plan::parse(&bytes).err(), Some(Error::InvalidElf));
    }
}

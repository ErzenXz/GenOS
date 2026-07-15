use core::ptr::copy_nonoverlapping;
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::prelude::Status;

const PT_LOAD: u32 = 1;
const EI_CLASS_64: u8 = 2;
const EI_DATA_LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;

#[derive(Clone, Copy)]
pub struct LoadedKernel {
    pub entry: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Header {
    ident: [u8; 16],
    elf_type: u16,
    machine: u16,
    version: u32,
    entry: u64,
    phoff: u64,
    shoff: u64,
    flags: u32,
    ehsize: u16,
    phentsize: u16,
    phnum: u16,
    shentsize: u16,
    shnum: u16,
    shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProgramHeader {
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

pub fn load_kernel(bytes: &[u8]) -> Result<LoadedKernel, Status> {
    let header = read_header(bytes)?;
    validate_header(header)?;

    let (image_start, image_end) = kernel_image_range(bytes, header)?;
    let pages = ((image_end - image_start) / 4096) as usize;
    boot::allocate_pages(
        AllocateType::Address(image_start),
        MemoryType::LOADER_CODE,
        pages,
    )
    .map_err(|e| e.status())?;

    for index in 0..header.phnum as usize {
        let offset = header.phoff as usize + index * header.phentsize as usize;
        let ph = read_program_header(bytes, offset)?;
        if ph.kind == PT_LOAD {
            load_segment(bytes, ph)?;
        }
    }

    Ok(LoadedKernel {
        entry: header.entry,
    })
}

fn kernel_image_range(bytes: &[u8], header: &Elf64Header) -> Result<(u64, u64), Status> {
    let mut start = u64::MAX;
    let mut end = 0u64;
    for index in 0..header.phnum as usize {
        let offset = header.phoff as usize + index * header.phentsize as usize;
        let ph = read_program_header(bytes, offset)?;
        if ph.kind != PT_LOAD {
            continue;
        }
        let seg_start = ph.paddr & !0xfff;
        let seg_end = (ph.paddr + ph.memsz + 0xfff) & !0xfff;
        start = start.min(seg_start);
        end = end.max(seg_end);
    }
    if start == u64::MAX || start >= end {
        return Err(Status::LOAD_ERROR);
    }
    Ok((start, end))
}

fn read_header(bytes: &[u8]) -> Result<&Elf64Header, Status> {
    if bytes.len() < core::mem::size_of::<Elf64Header>() {
        return Err(Status::LOAD_ERROR);
    }
    Ok(unsafe { &*(bytes.as_ptr().cast::<Elf64Header>()) })
}

fn validate_header(header: &Elf64Header) -> Result<(), Status> {
    if &header.ident[..4] != b"\x7fELF" {
        return Err(Status::LOAD_ERROR);
    }
    if header.ident[4] != EI_CLASS_64 || header.ident[5] != EI_DATA_LSB {
        return Err(Status::LOAD_ERROR);
    }
    if header.elf_type != ET_EXEC || header.machine != EM_X86_64 {
        return Err(Status::LOAD_ERROR);
    }
    if header.phentsize as usize != core::mem::size_of::<ProgramHeader>() {
        return Err(Status::LOAD_ERROR);
    }
    Ok(())
}

fn read_program_header(bytes: &[u8], offset: usize) -> Result<ProgramHeader, Status> {
    let end = offset
        .checked_add(core::mem::size_of::<ProgramHeader>())
        .ok_or(Status::LOAD_ERROR)?;
    if end > bytes.len() {
        return Err(Status::LOAD_ERROR);
    }
    Ok(unsafe { *(bytes.as_ptr().add(offset).cast::<ProgramHeader>()) })
}

fn load_segment(bytes: &[u8], ph: ProgramHeader) -> Result<(), Status> {
    let file_start = ph.offset as usize;
    let file_end = file_start
        .checked_add(ph.filesz as usize)
        .ok_or(Status::LOAD_ERROR)?;
    if file_end > bytes.len() || ph.memsz < ph.filesz {
        return Err(Status::LOAD_ERROR);
    }

    unsafe {
        let dst = ph.paddr as *mut u8;
        core::ptr::write_bytes(dst, 0, ph.memsz as usize);
        copy_nonoverlapping(bytes.as_ptr().add(file_start), dst, ph.filesz as usize);
    }

    Ok(())
}

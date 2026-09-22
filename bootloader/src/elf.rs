use bootloader::kernel_elf::Image;
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::prelude::Status;

#[derive(Clone, Copy)]
pub struct LoadedKernel {
    pub entry: u64,
}

pub fn load_kernel(bytes: &[u8]) -> Result<LoadedKernel, Status> {
    // No allocation or destination write occurs until every segment, entry,
    // source range and permission request has passed the bounded contract.
    let image = Image::parse(bytes).map_err(rejected)?;
    image
        .validate_source_address(bytes.as_ptr() as u64)
        .map_err(rejected)?;
    let destination = boot::allocate_pages(
        AllocateType::Address(image.image_start()),
        MemoryType::LOADER_CODE,
        image.pages(),
    )
    .map_err(|error| error.status())?;
    if destination.as_ptr() as u64 != image.image_start() {
        // SAFETY: firmware just returned this fresh allocation; it has not
        // been read, written, exposed, or used to construct any reference.
        let _ = unsafe { boot::free_pages(destination, image.pages()) };
        return Err(Status::LOAD_ERROR);
    }
    // SAFETY: successful exact-address UEFI allocation owns this whole bounded
    // page span and refuses overlap with live loader/firmware allocations.
    // Source collision was checked above; the immutable plan validates every
    // copy interval. Raw writes initialize all pages before any reference to
    // their contents exists. Ownership is retained for the kernel handoff.
    // The loader is single-threaded, before ExitBootServices and kernel IRQs.
    unsafe {
        core::ptr::write_bytes(destination.as_ptr(), 0, image.image_size());
        for (offset, file) in image.copies() {
            core::ptr::copy_nonoverlapping(
                file.as_ptr(),
                destination.as_ptr().add(offset),
                file.len(),
            );
        }
    }
    Ok(LoadedKernel {
        entry: image.entry(),
    })
}

fn rejected(error: bootloader::kernel_elf::Error) -> Status {
    uefi::println!("KERNEL_ELF_REJECTED reason={:?}", error);
    Status::LOAD_ERROR
}

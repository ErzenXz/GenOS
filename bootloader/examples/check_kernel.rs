//! Validate and simulate loading an existing kernel ELF without firmware.
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: check_kernel KERNEL.ELF");
    let bytes = std::fs::read(&path).expect("read kernel ELF");
    let image = bootloader::kernel_elf::Image::parse(&bytes).expect("validate kernel ELF");
    image
        .validate_source_address(bytes.as_ptr() as u64)
        .expect("disjoint source");
    let mut destination = vec![0xa5; image.image_size()];
    image
        .copy_into(&mut destination)
        .expect("simulate segment loading");
    println!(
        "KERNEL_ELF_VALID path={path} entry={:#x} base={:#x} bytes={} segments={}",
        image.entry(),
        image.image_start(),
        image.image_size(),
        image.segment_count()
    );
}

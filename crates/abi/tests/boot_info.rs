use genos_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

#[test]
fn boot_info_defaults_are_versioned() {
    let info = BootInfo::empty();
    assert_eq!(info.magic, BOOT_INFO_MAGIC);
    assert_eq!(info.version, BOOT_INFO_VERSION);
}

#[test]
fn cmdline_is_bounded() {
    let mut info = BootInfo::empty();
    info.set_cmdline("root=initrd console=fb");
    assert_eq!(info.cmdline_len, 22);
    assert_eq!(&info.cmdline[..4], b"root");
}

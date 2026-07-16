fn main() {
    let linker =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rustc-link-arg-bin=genos-init=-T{}", linker.display());
}

fn main() {
    println!("cargo:rerun-if-env-changed=GENOS_LIBFUZZER_ARCHIVE");
    if std::env::var_os("CARGO_FEATURE_RUNTIME").is_none() {
        return;
    }
    let archive = std::path::PathBuf::from(
        std::env::var_os("GENOS_LIBFUZZER_ARCHIVE")
            .expect("use tools/test_coverage_fuzz.py to build the pinned runtime"),
    );
    assert!(archive.is_absolute() && archive.is_file());
    assert_eq!(archive.file_name().unwrap(), "libFuzzer.a");
    println!("cargo:rerun-if-changed={}", archive.display());
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static=Fuzzer");
    let target = std::env::var("TARGET").unwrap();
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-lib=c++");
    } else if target.contains("linux") {
        println!("cargo:rustc-link-lib=stdc++");
    } else {
        panic!("coverage fuzz runtime supports Darwin and Linux hosts only");
    }
}

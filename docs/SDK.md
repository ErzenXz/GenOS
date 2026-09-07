# GenOS standalone application SDK

The SDK builds an actual x86_64 GenOS ELF outside the operating-system repository. It includes an offline copy of the public ABI and `no_std` syscall runtime, an application template, a linker script, and the MIT license. It requires the exported, pinned Rust 1.97.0 toolchain and the `x86_64-unknown-none` target.

From the GenOS checkout:

```sh
cargo xtask new-app /absolute/path/to/my-genos-app
```

The destination must not already exist, and its parent must exist. From that new directory:

```sh
rustup target add x86_64-unknown-none
cargo build --release --offline
```

The executable is `target/x86_64-unknown-none/release/genos-app`. All dependencies are local to the exported workspace; moving the directory does not require the GenOS checkout. Keep the vendored ABI and runtime together when updating the SDK.

## Current application contract

- ABI 18 and image layout 2; startup checks the running kernel ABI and exits with status 78 on a mismatch. There is no promise that a different ABI version will accept this executable.
- A fixed 4 KiB executable page and 4 KiB writable data page for this small template, with linker assertions. The kernel supplies a separate guarded stack and validates W^X and every mapped page.
- `UserProcessHeader` occupies the first 16 bytes of the writable page. Its fields belong to the kernel.
- No allocator, C library, dynamic linker, POSIX compatibility, package installation, or automatic network authority is provided by this template.
- The example writes a bounded greeting through a validated pointer syscall and exits. A panic exits with status 101.

## End-to-end verification

From the GenOS checkout, `cargo xtask test-sdk` exports to a fresh system temporary directory, builds offline outside the repository, packages the resulting ELF as `SDK.ELF`, and boots it through the actual UEFI/kernel path. The test requires the exact greeting, successful Ring 3 exit, and address-space reclamation before the normal shell becomes ready. The output log is `build/serial-sdk.log`.

`cargo xtask test` includes this gate. Ordinary `cargo xtask build` images omit the SDK test application. On completion, the test rebuilds the normal production image. The external source workspace is retained and its path printed for inspection.

This establishes external build and isolated execution. General named application launch from the shell, manifests, signed packages, repository distribution, and a stable SDK release process remain roadmap work. Do not replace `INIT.ELF` with an arbitrary application: it is still used by the kernel's lifecycle acceptance probes.

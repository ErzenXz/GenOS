# Dormant framebuffer ownership and unsafe boundary

The serial-first boot does not construct a framebuffer device. The retained
renderer previously exposed a safe constructor taking only `FramebufferInfo`:
any safe Rust caller could supply an arbitrary base, then trigger a raw write.
Separate device instances also addressed one mutable global backbuffer. These
were unchecked mapping/ownership obligations behind a safe interface.

`FramebufferDevice<'a>` now has two explicit construction paths. `from_pixels`
borrows an exclusive caller-owned `&mut [u32]` target and optional separate
backbuffer for the device lifetime. `from_mmio` is unsafe and accepts the firmware
layout plus an optional exclusively borrowed backbuffer. No mutable global
backbuffer or stored raw drawing-buffer pointer remains. `DisplayManager` carries
the device lifetime; text and recovery-shell functions borrow it. The serial
composition root still constructs neither path. The existing headless-source
check now rejects any direct `FramebufferDevice::` call in that root.

Both constructors reject zero/out-of-i32 dimensions, stride shorter than width,
unsupported Bitmask/Unknown layouts, overflowing or over-isize byte extents, and
insufficient supplied buffers. MMIO admission additionally rejects zero/unaligned
bases, truncated/overflowing declared regions and overlap with the supplied Rust
backbuffer. These numeric checks do not grant access to an address or establish
hardware ownership. RGB/BGR use four-byte pixels with a zero reserved byte;
row padding and trailing caller storage are not drawn or presented.

The MMIO caller must retain an exclusively owned, writable mapping that permits
non-trapping aligned 32-bit volatile stores for the entire device lifetime,
including all aliases, interrupt/emergency contexts and other CPUs. It must be
external device memory; writes may not modify Rust allocations through device
side effects. The device is neither Send nor Sync, with no unsafe trait override.
No Rust reference to MMIO memory is fabricated. A future graphical composition
root must supply and review these obligations before calling the unsafe constructor.

Three unsafe constructs have scoped obligations:

- The unsafe MMIO constructor transfers the mapping/device-lifetime obligations
  above to the owned device; there is currently no production caller.
- The borrowed-RAM target's volatile store uses an aligned exclusive slice element
  with its original allocation provenance. The slice borrow remains live and the
  element bounds are checked before the write.
- The MMIO target checks the pixel index, computes an admitted non-wrapping aligned
  address and performs only a volatile store using a pointer without allocation
  provenance. This relies on the constructor's external-device contract; ordinary
  pointer arithmetic or references must not replace it.

These follow Rust's [volatile-write contract](https://doc.rust-lang.org/core/ptr/fn.write_volatile.html).
Volatile access supplies observable I/O behavior, not atomicity or synchronization.
It does not make concurrent or aliased device access safe.

Device-boundary clipping computes endpoints in wide integers before narrowing to
in-range pixels. Rectangle, blit, circle, checker and stroke offsets do not use
wrapping i32 address arithmetic. Line clipping bounds the raster segment before
Bresenham iteration, with wide interpolation and explicit even-brush edges.
Only a bounded admitted pixel index reaches either target. Arbitrarily expensive
drawing workloads and upstream presentation geometry still need resource/latency
qualification; this change does not establish a usable or hardened graphical path.

Ten host/Miri tests exercise layout and numeric-MMIO admission without accessing
MMIO, short buffers, RGB/BGR byte packing, buffered presentation/overlay restore,
stride/tail canaries, extreme rectangles/blits/lines/circles, independent surfaces
and existing small-surface wallpaper/checker/stroke/overlay operations. Four new
compile-fail checks deny aliased borrowed buffers, storage outliving its owner,
safe MMIO construction and thread transfer. Miri executes RAM-backed volatile
writes with provenance checks; it does not execute or qualify real MMIO.

The full serial/network/storage/system suite is a regression gate for the linked
kernel and lifetime propagation, not framebuffer hardware evidence. Actual GOP
mapping, physical display behavior, arbitrary raster workloads and Stage 9's
userspace graphics remain unqualified. Wider F4/F5 ownership and unsafe review
also remain open.

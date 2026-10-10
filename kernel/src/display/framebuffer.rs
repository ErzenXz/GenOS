use core::marker::PhantomData;

use genos_abi::{FramebufferInfo, PixelFormat};

use super::{Color, Point, Rect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramebufferError {
    Dimensions,
    PixelFormat,
    Extent,
    TargetTooSmall,
    BackbufferTooSmall,
    BackbufferAlias,
}

struct Layout {
    width: i32,
    height: i32,
    stride: usize,
    pixels: usize,
    bytes: usize,
    format: PixelFormat,
}

impl Layout {
    fn new(
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Result<Self, FramebufferError> {
        if width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || stride < width
        {
            return Err(FramebufferError::Dimensions);
        }
        if !matches!(format, PixelFormat::Rgb | PixelFormat::Bgr) {
            return Err(FramebufferError::PixelFormat);
        }
        let pixels = (stride as usize)
            .checked_mul(height as usize)
            .ok_or(FramebufferError::Extent)?;
        let bytes = pixels
            .checked_mul(4)
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or(FramebufferError::Extent)?;
        Ok(Self {
            width: width as i32,
            height: height as i32,
            stride: stride as usize,
            pixels,
            bytes,
            format,
        })
    }
}

fn mmio_range(info: &FramebufferInfo) -> Result<(Layout, usize), FramebufferError> {
    let layout = Layout::new(info.width, info.height, info.stride, info.pixel_format)?;
    let address = usize::try_from(info.base).map_err(|_| FramebufferError::Extent)?;
    if address == 0
        || !address.is_multiple_of(4)
        || info.size < layout.bytes as u64
        || info.base.checked_add(info.size).is_none()
        || address.checked_add(layout.bytes).is_none()
    {
        return Err(FramebufferError::Extent);
    }
    Ok((layout, address))
}

fn reject_backbuffer_alias(
    address: usize,
    bytes: usize,
    buffer: Option<&[u32]>,
) -> Result<(), FramebufferError> {
    if let Some(buffer) = buffer {
        let start = buffer.as_ptr().addr();
        let end = start
            .checked_add(core::mem::size_of_val(buffer))
            .ok_or(FramebufferError::Extent)?;
        if start < address + bytes && address < end {
            return Err(FramebufferError::BackbufferAlias);
        }
    }
    Ok(())
}

enum Target<'a> {
    Pixels(&'a mut [u32]),
    Mmio { address: usize, pixels: usize },
}

impl Target<'_> {
    fn write(&mut self, index: usize, value: u32) {
        match self {
            Self::Pixels(pixels) => {
                if let Some(pixel) = pixels.get_mut(index) {
                    // SAFETY: this exclusive, aligned element reference retains
                    // allocation provenance and is live for this write. Volatile
                    // does not grant synchronization or additional alias rights.
                    unsafe { core::ptr::write_volatile(pixel, value) };
                }
            }
            Self::Mmio { address, pixels } if index < *pixels => {
                let address = *address + index * 4;
                // SAFETY: unsafe construction requires this entire external-MMIO
                // range mapped, exclusively owned and non-trapping for u32 stores
                // for the device lifetime. Checked layout/extent and index keep
                // this aligned address in range. No Rust reference is fabricated.
                unsafe { core::ptr::without_provenance_mut::<u32>(address).write_volatile(value) };
            }
            Self::Mmio { .. } => {}
        }
    }
}

pub struct FramebufferDevice<'a> {
    target: Target<'a>,
    draw: Option<&'a mut [u32]>,
    width: i32,
    height: i32,
    stride: usize,
    format: PixelFormat,
    draw_len: usize,
    // MMIO is a BSP-owned device, not transferable or shared across threads.
    _single_thread: PhantomData<*mut ()>,
}

impl<'a> FramebufferDevice<'a> {
    /// Build a safe RAM surface from exclusive caller-owned pixel storage.
    /// A supplied backbuffer must cover every stride row, including padding.
    ///
    /// ```compile_fail
    /// use kernel::display::FramebufferDevice;
    /// use genos_abi::PixelFormat;
    /// let mut pixels = [0u32; 4];
    /// let fb = FramebufferDevice::from_pixels(2, 2, 2, PixelFormat::Rgb,
    ///     &mut pixels, Some(&mut pixels));
    /// ```
    ///
    /// ```compile_fail
    /// use kernel::display::FramebufferDevice;
    /// use genos_abi::PixelFormat;
    /// let mut fb = {
    ///     let mut pixels = [0u32; 4];
    ///     FramebufferDevice::from_pixels(2, 2, 2, PixelFormat::Rgb,
    ///         &mut pixels, None).unwrap()
    /// };
    /// fb.present_all();
    /// ```
    pub fn from_pixels(
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
        pixels: &'a mut [u32],
        backbuffer: Option<&'a mut [u32]>,
    ) -> Result<Self, FramebufferError> {
        let layout = Layout::new(width, height, stride, format)?;
        if pixels.len() < layout.pixels {
            return Err(FramebufferError::TargetTooSmall);
        }
        Self::build(Target::Pixels(pixels), backbuffer, layout)
    }

    /// Admit numeric layout and construct an external-MMIO drawing device.
    ///
    /// # Safety
    /// The touched range must remain mapped and writable without trapping for
    /// aligned 32-bit volatile stores until this device is dropped. It must be
    /// outside every Rust allocation; writes must not modify Rust-allocated
    /// memory through hardware side effects. The caller exclusively owns that
    /// hardware range, including all aliases, IRQ/NMI paths and other CPUs, and
    /// must not create another device or reference accessing it concurrently.
    /// The optional backbuffer is ordinary exclusively borrowed Rust memory.
    /// Numeric admission cannot establish mapping, device or alias ownership.
    ///
    /// ```compile_fail
    /// use kernel::display::FramebufferDevice;
    /// use genos_abi::FramebufferInfo;
    /// let device = FramebufferDevice::from_mmio(&FramebufferInfo::empty(), None);
    /// ```
    ///
    /// ```compile_fail
    /// use kernel::display::FramebufferDevice;
    /// fn require_send<T: Send>() {}
    /// require_send::<FramebufferDevice<'static>>();
    /// ```
    pub unsafe fn from_mmio(
        info: &FramebufferInfo,
        backbuffer: Option<&'a mut [u32]>,
    ) -> Result<Self, FramebufferError> {
        let (layout, address) = mmio_range(info)?;
        reject_backbuffer_alias(address, layout.bytes, backbuffer.as_deref())?;
        Self::build(
            Target::Mmio {
                address,
                pixels: layout.pixels,
            },
            backbuffer,
            layout,
        )
    }

    fn build(
        target: Target<'a>,
        draw: Option<&'a mut [u32]>,
        layout: Layout,
    ) -> Result<Self, FramebufferError> {
        if draw
            .as_ref()
            .is_some_and(|buffer| buffer.len() < layout.pixels)
        {
            return Err(FramebufferError::BackbufferTooSmall);
        }
        Ok(Self {
            target,
            draw,
            width: layout.width,
            height: layout.height,
            stride: layout.stride,
            format: layout.format,
            draw_len: layout.bytes,
            _single_thread: PhantomData,
        })
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn is_backbuffered(&self) -> bool {
        self.draw.is_some()
    }

    pub fn draw_bytes_len(&self) -> usize {
        self.draw_len
    }

    pub fn present_all(&mut self) {
        self.present_rect(self.bounds());
    }

    pub fn present_rect(&mut self, rect: Rect) {
        let clipped = self.clip(rect);
        let Some(draw) = self.draw.as_ref() else {
            return;
        };
        for y in clipped.y..clipped.bottom() {
            for x in clipped.x..clipped.right() {
                let index = y as usize * self.stride + x as usize;
                self.target.write(index, draw[index]);
            }
        }
    }

    pub fn overlay_line(
        &mut self,
        start: Point,
        end: Point,
        thickness: i32,
        color: Color,
        clip: Rect,
    ) {
        self.draw_line(start, end, thickness, color, clip, true);
    }

    pub fn overlay_fill_rect(&mut self, rect: Rect, color: Color) {
        let clipped = self.clip(rect);
        for y in clipped.y..clipped.bottom() {
            for x in clipped.x..clipped.right() {
                self.put_physical_pixel(x, y, color);
            }
        }
    }

    pub fn blit_pixels(&mut self, dest: Point, size: super::Size, pixels: &[Color]) {
        if size.width <= 0 || size.height <= 0 {
            return;
        }
        let rect = self.clip(Rect::new(dest.x, dest.y, size.width, size.height));
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let source_x = i64::from(x) - i64::from(dest.x);
                let source_y = i64::from(y) - i64::from(dest.y);
                let index = source_y * i64::from(size.width) + source_x;
                if let Ok(index) = usize::try_from(index) {
                    if let Some(color) = pixels.get(index).copied() {
                        self.put_pixel(x, y, color);
                    }
                }
            }
        }
    }

    pub fn blit_solid(&mut self, rect: Rect, color: Color) {
        self.fill_rect(rect, color);
    }

    pub fn clear(&mut self, color: Color) {
        self.fill_rect(self.bounds(), color);
    }

    pub fn desktop_wallpaper(&mut self) {
        self.desktop_wallpaper_rect(self.bounds());
    }

    pub fn desktop_wallpaper_rect(&mut self, rect: Rect) {
        let bounds = self.bounds();
        let clipped = self.clip(rect);
        self.fill_rect(clipped, Color::rgb(40, 42, 40));

        let mut x = 0;
        while x < bounds.width {
            self.fill_rect(
                Rect::new(x, 0, 1, bounds.height).intersect(clipped),
                Color::rgb(46, 48, 46),
            );
            x = x.saturating_add(64);
        }
        let mut y = 0;
        while y < bounds.height {
            self.fill_rect(
                Rect::new(0, y, bounds.width, 1).intersect(clipped),
                Color::rgb(46, 48, 46),
            );
            y = y.saturating_add(64);
        }

        self.fill_rect(
            Rect::new(0, 0, 128, bounds.height).intersect(clipped),
            Color::rgb(34, 36, 34),
        );
        self.fill_rect(
            Rect::new(127, 0, 1, bounds.height).intersect(clipped),
            Color::rgb(67, 68, 64),
        );
        self.fill_rect(
            Rect::new(bounds.width / 2 - 1, 0, 2, bounds.height).intersect(clipped),
            Color::rgb(43, 45, 43),
        );
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let clipped = self.clip(rect);
        for y in clipped.y..clipped.bottom() {
            for x in clipped.x..clipped.right() {
                self.put_pixel(x, y, color);
            }
        }
    }

    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if rect.is_empty() {
            return;
        }
        let (x, y, width, height) = (
            i64::from(rect.x),
            i64::from(rect.y),
            i64::from(rect.width),
            i64::from(rect.height),
        );
        for (x, y, width, height) in [
            (x, y, width, 1),
            (x, y + height - 1, width, 1),
            (x, y, 1, height),
            (x + width - 1, y, 1, height),
        ] {
            let clipped = self.clip_box(x, y, width, height);
            self.fill_rect(clipped, color);
        }
    }

    pub fn fill_rect_checker(&mut self, rect: Rect, primary: Color, secondary: Color, cell: i32) {
        let clipped = self.clip(rect);
        let cell = cell.max(1);
        for y in clipped.y..clipped.bottom() {
            for x in clipped.x..clipped.right() {
                let color = if ((i64::from(x) / i64::from(cell)) + (i64::from(y) / i64::from(cell)))
                    % 2
                    == 0
                {
                    primary
                } else {
                    secondary
                };
                self.put_pixel(x, y, color);
            }
        }
    }

    pub fn fill_rect_checker_blocks(
        &mut self,
        rect: Rect,
        primary: Color,
        secondary: Color,
        cell: i32,
    ) {
        let clipped = self.clip(rect);
        if clipped.is_empty() {
            return;
        }
        let cell = cell.max(1);
        let mut y = clipped.y;
        while y < clipped.bottom() {
            let height = cell.min(clipped.bottom() - y);
            let mut x = clipped.x;
            while x < clipped.right() {
                let width = cell.min(clipped.right() - x);
                let color = if ((i64::from(x) / i64::from(cell)) + (i64::from(y) / i64::from(cell)))
                    % 2
                    == 0
                {
                    primary
                } else {
                    secondary
                };
                self.fill_rect(Rect::new(x, y, width, height), color);
                x += cell;
            }
            y += cell;
        }
    }

    pub fn fill_circle(&mut self, center: Point, radius: i32, color: Color) {
        if radius < 0 {
            return;
        }
        let radius = i64::from(radius);
        let rect = self.clip_box(
            i64::from(center.x) - radius,
            i64::from(center.y) - radius,
            radius * 2 + 1,
            radius * 2 + 1,
        );
        let squared = i128::from(radius) * i128::from(radius);
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let dx = i128::from(x) - i128::from(center.x);
                let dy = i128::from(y) - i128::from(center.y);
                if dx * dx + dy * dy <= squared {
                    self.put_pixel(x, y, color);
                }
            }
        }
    }

    pub fn line(&mut self, start: Point, end: Point, thickness: i32, color: Color, clip: Rect) {
        self.draw_line(start, end, thickness, color, clip, false);
    }
    fn clip(&self, rect: Rect) -> Rect {
        self.clip_box(
            i64::from(rect.x),
            i64::from(rect.y),
            i64::from(rect.width),
            i64::from(rect.height),
        )
    }

    fn clip_box(&self, x: i64, y: i64, width: i64, height: i64) -> Rect {
        if width <= 0 || height <= 0 {
            return Rect::new(0, 0, 0, 0);
        }
        let left = x.max(0);
        let top = y.max(0);
        let right = (x + width).min(i64::from(self.width));
        let bottom = (y + height).min(i64::from(self.height));
        if left >= right || top >= bottom {
            return Rect::new(0, 0, 0, 0);
        }
        Rect::new(
            left as i32,
            top as i32,
            (right - left) as i32,
            (bottom - top) as i32,
        )
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.width && y < self.height)
            .then(|| y as usize * self.stride + x as usize)
    }

    fn packed(&self, color: Color) -> u32 {
        u32::from_ne_bytes(match self.format {
            PixelFormat::Rgb => [color.r, color.g, color.b, 0],
            PixelFormat::Bgr => [color.b, color.g, color.r, 0],
            _ => unreachable!("only admitted RGB/BGR layouts are constructed"),
        })
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        let Some(index) = self.index(x, y) else {
            return;
        };
        let packed = self.packed(color);
        if let Some(draw) = self.draw.as_mut() {
            draw[index] = packed;
        } else {
            self.target.write(index, packed);
        }
    }

    fn put_physical_pixel(&mut self, x: i32, y: i32, color: Color) {
        if let Some(index) = self.index(x, y) {
            self.target.write(index, self.packed(color));
        }
    }

    fn draw_line(
        &mut self,
        start: Point,
        end: Point,
        thickness: i32,
        color: Color,
        clip: Rect,
        overlay: bool,
    ) {
        let clip = self.clip(clip);
        if clip.is_empty() {
            return;
        }
        let thickness = i64::from(thickness.max(1));
        let radius = (thickness - 1) / 2;
        let forward = thickness - 1 - radius;
        let (x, y, end_x, end_y) = (
            i64::from(start.x),
            i64::from(start.y),
            i64::from(end.x),
            i64::from(end.y),
        );
        let dx = (end_x - x).abs();
        let dy = (end_y - y).abs();
        let sx = if x < end_x { 1 } else { -1 };
        let sy = if y < end_y { 1 } else { -1 };
        let major = dx.max(dy);
        let steps = if dx >= dy {
            visible_steps(
                x,
                sx,
                major,
                i64::from(clip.x) - forward,
                i64::from(clip.right()) - 1 + radius,
            )
        } else {
            visible_steps(
                y,
                sy,
                major,
                i64::from(clip.y) - forward,
                i64::from(clip.bottom()) - 1 + radius,
            )
        };
        let Some(steps) = steps else {
            return;
        };
        for step in steps {
            // This closed form is the original symmetric Bresenham phase,
            // including half-step ties, even when the leading steps are skipped.
            // i128 covers products of any two full-range i32 endpoint deltas.
            let (x, y) = if major == 0 {
                (x, y)
            } else if dx >= dy {
                (
                    x + sx * step,
                    y + sy
                        * ((i128::from(dy) * i128::from(step) + i128::from(dx / 2))
                            / i128::from(dx)) as i64,
                )
            } else {
                (
                    x + sx
                        * ((i128::from(dx) * i128::from(step) + i128::from(dy / 2))
                            / i128::from(dy)) as i64,
                    y + sy * step,
                )
            };
            let brush = self
                .clip_box(x - radius, y - radius, thickness, thickness)
                .intersect(clip);
            if overlay {
                self.overlay_fill_rect(brush, color);
            } else {
                self.fill_rect(brush, color);
            }
        }
    }
}

fn visible_steps(
    start: i64,
    direction: i64,
    major: i64,
    low: i64,
    high: i64,
) -> Option<core::ops::RangeInclusive<i64>> {
    let (first, last) = if direction > 0 {
        (low - start, high - start)
    } else {
        (start - high, start - low)
    };
    let first = first.max(0);
    let last = last.min(major);
    (first <= last).then_some(first..=last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Size;

    const SENTINEL: u32 = 0xdeadbeef;
    const RED: Color = Color::rgb(0x11, 0x22, 0x33);
    fn packed(color: Color, format: PixelFormat) -> u32 {
        u32::from_ne_bytes(if format == PixelFormat::Rgb {
            [color.r, color.g, color.b, 0]
        } else {
            [color.b, color.g, color.r, 0]
        })
    }

    #[test]
    fn rejects_invalid_layouts_and_short_owned_buffers() {
        for (width, height, stride) in [
            (0, 1, 1),
            (1, 0, 1),
            (2, 1, 1),
            (u32::MAX, 1, u32::MAX),
            (1, u32::MAX, 1),
        ] {
            assert!(matches!(
                Layout::new(width, height, stride, PixelFormat::Rgb),
                Err(FramebufferError::Dimensions)
            ));
        }
        for format in [PixelFormat::Bitmask, PixelFormat::Unknown] {
            assert!(matches!(
                Layout::new(1, 1, 1, format),
                Err(FramebufferError::PixelFormat)
            ));
        }
        assert!(matches!(
            Layout::new(i32::MAX as u32, i32::MAX as u32, u32::MAX, PixelFormat::Rgb),
            Err(FramebufferError::Extent)
        ));
        let mut pixels = [0; 4];
        let mut draw = [0; 3];
        assert!(matches!(
            FramebufferDevice::from_pixels(2, 2, 3, PixelFormat::Rgb, &mut pixels, None),
            Err(FramebufferError::TargetTooSmall)
        ));
        assert!(matches!(
            FramebufferDevice::from_pixels(2, 2, 2, PixelFormat::Rgb, &mut pixels, Some(&mut draw)),
            Err(FramebufferError::BackbufferTooSmall)
        ));
    }

    #[test]
    fn mmio_numeric_admission_never_accesses_the_address() {
        let good = FramebufferInfo {
            base: 0x1000,
            size: 32,
            width: 2,
            height: 2,
            stride: 4,
            pixel_format: PixelFormat::Bgr,
        };
        let (layout, address) = mmio_range(&good).unwrap();
        assert_eq!((layout.pixels, layout.bytes, address), (8, 32, 0x1000));
        let buffer = [0u32; 8];
        let start = buffer.as_ptr().addr();
        assert_eq!(
            reject_backbuffer_alias(start, 32, Some(&buffer)),
            Err(FramebufferError::BackbufferAlias)
        );
        assert_eq!(
            reject_backbuffer_alias(start + 32, 32, Some(&buffer)),
            Ok(())
        );
        assert_eq!(
            reject_backbuffer_alias(start - 32, 32, Some(&buffer)),
            Ok(())
        );
        assert_eq!(reject_backbuffer_alias(start, 32, None), Ok(()));
        for info in [
            FramebufferInfo { base: 0, ..good },
            FramebufferInfo {
                base: 0x1001,
                ..good
            },
            FramebufferInfo { size: 31, ..good },
            FramebufferInfo {
                base: u64::MAX - 3,
                ..good
            },
            FramebufferInfo {
                size: u64::MAX,
                ..good
            },
        ] {
            assert!(matches!(mmio_range(&info), Err(FramebufferError::Extent)));
        }
    }

    #[test]
    fn direct_rgb_and_bgr_writes_preserve_padding_and_tail_canaries() {
        for format in [PixelFormat::Rgb, PixelFormat::Bgr] {
            let mut pixels = [SENTINEL; 10];
            {
                let mut fb =
                    FramebufferDevice::from_pixels(2, 2, 4, format, &mut pixels, None).unwrap();
                assert!(!fb.is_backbuffered());
                assert_eq!(fb.draw_bytes_len(), 32);
                fb.clear(RED);
                fb.present_all();
            }
            for (index, value) in pixels.iter().enumerate() {
                assert_eq!(
                    *value,
                    if [0, 1, 4, 5].contains(&index) {
                        packed(RED, format)
                    } else {
                        SENTINEL
                    }
                );
            }
        }
    }

    #[test]
    fn buffered_present_overlay_and_restore_have_separate_ownership() {
        let mut pixels = [SENTINEL; 10];
        let mut draw = [0xabcdef01; 10];
        {
            let mut fb = FramebufferDevice::from_pixels(
                2,
                2,
                4,
                PixelFormat::Rgb,
                &mut pixels,
                Some(&mut draw),
            )
            .unwrap();
            fb.clear(RED);
            fb.present_rect(Rect::new(0, 0, 1, 1));
            if let Target::Pixels(target) = &fb.target {
                assert_eq!(target[0], packed(RED, PixelFormat::Rgb));
                assert_eq!(target[1], SENTINEL);
            }
            fb.overlay_fill_rect(Rect::new(0, 0, 1, 1), Color::BLACK);
            assert_eq!(fb.draw.as_ref().unwrap()[0], packed(RED, PixelFormat::Rgb));
            fb.present_all();
        }
        for index in [0, 1, 4, 5] {
            assert_eq!(pixels[index], packed(RED, PixelFormat::Rgb));
        }
        for index in [2, 3, 6, 7, 8, 9] {
            assert_eq!(pixels[index], SENTINEL);
            assert_eq!(draw[index], 0xabcdef01);
        }
    }

    #[test]
    fn extreme_rectangles_clip_without_overflow_or_outside_writes() {
        let mut pixels = [SENTINEL; 12];
        {
            let mut fb =
                FramebufferDevice::from_pixels(3, 2, 5, PixelFormat::Rgb, &mut pixels, None)
                    .unwrap();
            for rect in [
                Rect::new(i32::MAX, i32::MAX, i32::MAX, i32::MAX),
                Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
                Rect::new(0, 0, -1, 2),
                Rect::new(0, 0, 2, -1),
            ] {
                fb.fill_rect(rect, RED);
                fb.stroke_rect(rect, RED);
                fb.present_rect(rect);
            }
            fb.fill_rect(Rect::new(-1, -1, i32::MAX, i32::MAX), RED);
        }
        for (index, pixel) in pixels.iter().enumerate() {
            assert_eq!(
                *pixel,
                if [0, 1, 2, 5, 6, 7].contains(&index) {
                    packed(RED, PixelFormat::Rgb)
                } else {
                    SENTINEL
                }
            );
        }
    }

    #[test]
    fn clipped_blits_preserve_source_offsets_and_short_input() {
        let colors = [
            Color::BLACK,
            RED,
            Color::SUCCESS,
            Color::WARNING,
            Color::ERROR,
            Color::ACCENT,
        ];
        let mut pixels = [SENTINEL; 6];
        {
            let mut fb =
                FramebufferDevice::from_pixels(2, 2, 3, PixelFormat::Bgr, &mut pixels, None)
                    .unwrap();
            fb.blit_pixels(Point::new(-1, 0), Size::new(3, 2), &colors);
            fb.blit_pixels(Point::new(i32::MIN, 0), Size::new(i32::MAX, 1), &colors);
            fb.blit_pixels(Point::new(0, 0), Size::new(2, 2), &[]);
        }
        assert_eq!(
            pixels,
            [
                packed(colors[1], PixelFormat::Bgr),
                packed(colors[2], PixelFormat::Bgr),
                SENTINEL,
                packed(colors[4], PixelFormat::Bgr),
                packed(colors[5], PixelFormat::Bgr),
                SENTINEL
            ]
        );
    }

    #[test]
    fn lines_clip_extreme_endpoints_and_even_brush_edges() {
        let mut pixels = [SENTINEL; 20];
        {
            let mut fb =
                FramebufferDevice::from_pixels(4, 4, 5, PixelFormat::Rgb, &mut pixels, None)
                    .unwrap();
            fb.line(
                Point::new(i32::MIN, 1),
                Point::new(i32::MAX, 1),
                1,
                RED,
                fb.bounds(),
            );
            fb.line(Point::new(-1, 0), Point::new(-1, 3), 2, RED, fb.bounds());
            fb.line(
                Point::new(i32::MIN, i32::MIN),
                Point::new(i32::MIN, i32::MAX),
                1,
                RED,
                fb.bounds(),
            );
        }
        for y in 0..4 {
            for x in 0..5 {
                assert_eq!(
                    pixels[y * 5 + x],
                    if x < 4 && (y == 1 || x == 0) {
                        packed(RED, PixelFormat::Rgb)
                    } else {
                        SENTINEL
                    }
                );
            }
        }
        assert_eq!(visible_steps(0, 1, 3, 0, 3), Some(0..=3));
        assert!(visible_steps(-2, 1, 1, 0, 3).is_none());
    }

    #[test]
    fn circles_use_wide_arithmetic_and_preserve_row_padding() {
        let mut pixels = [SENTINEL; 12];
        {
            let mut fb =
                FramebufferDevice::from_pixels(3, 3, 4, PixelFormat::Rgb, &mut pixels, None)
                    .unwrap();
            fb.fill_circle(Point::new(i32::MIN, i32::MIN), i32::MAX, RED);
            fb.fill_circle(Point::new(1, 1), -1, RED);
            fb.fill_circle(Point::new(1, 1), 1, RED);
        }
        for y in 0..3 {
            for x in 0..4 {
                let circle = x < 3 && (x == 1 || y == 1);
                assert_eq!(
                    pixels[y * 4 + x],
                    if circle {
                        packed(RED, PixelFormat::Rgb)
                    } else {
                        SENTINEL
                    }
                );
            }
        }
    }

    #[test]
    fn independent_surfaces_do_not_share_a_backbuffer() {
        let mut a = [0; 4];
        let mut b = [0; 4];
        let mut draw_a = [0; 4];
        let mut draw_b = [0; 4];
        {
            let mut first = FramebufferDevice::from_pixels(
                2,
                2,
                2,
                PixelFormat::Rgb,
                &mut a,
                Some(&mut draw_a),
            )
            .unwrap();
            let mut second = FramebufferDevice::from_pixels(
                2,
                2,
                2,
                PixelFormat::Rgb,
                &mut b,
                Some(&mut draw_b),
            )
            .unwrap();
            first.clear(RED);
            second.clear(Color::BLACK);
            second.present_all();
            first.present_all();
        }
        assert_eq!(a, [packed(RED, PixelFormat::Rgb); 4]);
        assert_eq!(b, [0; 4]);
    }

    #[test]
    fn checker_wallpaper_and_overlay_line_stay_inside_small_surfaces() {
        let mut pixels = [SENTINEL; 30];
        let mut draw = [SENTINEL; 30];
        {
            let mut fb = FramebufferDevice::from_pixels(
                4,
                4,
                6,
                PixelFormat::Rgb,
                &mut pixels,
                Some(&mut draw),
            )
            .unwrap();
            fb.desktop_wallpaper();
            fb.fill_rect_checker(Rect::new(-1, -1, 8, 8), RED, Color::BLACK, 0);
            fb.fill_rect_checker_blocks(Rect::new(-1, -1, 8, 8), RED, Color::BLACK, 1);
            fb.stroke_rect(Rect::new(0, 0, 4, 4), RED);
            fb.present_all();
            fb.overlay_line(
                Point::new(0, 0),
                Point::new(3, 3),
                1,
                Color::WARNING,
                fb.bounds(),
            );
        }
        for y in 0..4 {
            for x in 4..6 {
                assert_eq!(pixels[y * 6 + x], SENTINEL);
                assert_eq!(draw[y * 6 + x], SENTINEL);
            }
        }
        for index in 24..30 {
            assert_eq!(pixels[index], SENTINEL);
            assert_eq!(draw[index], SENTINEL);
        }
    }
    #[test]
    fn clipped_line_phase_matches_original_raster_for_all_small_endpoints() {
        for start_x in -2i32..6 {
            for start_y in -2i32..6 {
                for end_x in -2i32..6 {
                    for end_y in -2i32..6 {
                        let mut expected = [SENTINEL; 20];
                        // Independent replay of the original iterative rasterizer.
                        let (mut x, mut y) = (start_x, start_y);
                        let dx = (end_x - x).abs();
                        let sx = if x < end_x { 1 } else { -1 };
                        let dy = -(end_y - y).abs();
                        let sy = if y < end_y { 1 } else { -1 };
                        let mut error = dx + dy;
                        loop {
                            if (0..4).contains(&x) && (0..4).contains(&y) {
                                expected[y as usize * 5 + x as usize] =
                                    packed(RED, PixelFormat::Rgb);
                            }
                            if x == end_x && y == end_y {
                                break;
                            }
                            let doubled = error * 2;
                            if doubled >= dy {
                                error += dy;
                                x += sx;
                            }
                            if doubled <= dx {
                                error += dx;
                                y += sy;
                            }
                        }
                        let mut actual = [SENTINEL; 20];
                        {
                            let mut fb = FramebufferDevice::from_pixels(
                                4,
                                4,
                                5,
                                PixelFormat::Rgb,
                                &mut actual,
                                None,
                            )
                            .unwrap();
                            fb.line(
                                Point::new(start_x, start_y),
                                Point::new(end_x, end_y),
                                1,
                                RED,
                                fb.bounds(),
                            );
                        }
                        assert_eq!(
                            actual, expected,
                            "start=({start_x},{start_y}) end=({end_x},{end_y})"
                        );
                    }
                }
            }
        }
    }
}

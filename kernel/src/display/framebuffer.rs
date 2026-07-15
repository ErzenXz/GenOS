use genos_abi::{FramebufferInfo, PixelFormat};

use super::{Color, Point, Rect};

const MAX_BACKBUFFER_WIDTH: usize = 1920;
const MAX_BACKBUFFER_HEIGHT: usize = 1080;
const MAX_BACKBUFFER_BYTES: usize = MAX_BACKBUFFER_WIDTH * MAX_BACKBUFFER_HEIGHT * 4;

static mut BACKBUFFER: [u8; MAX_BACKBUFFER_BYTES] = [0; MAX_BACKBUFFER_BYTES];

pub struct FramebufferDevice {
    base: *mut u8,
    draw_base: *mut u8,
    draw_len: usize,
    width: i32,
    height: i32,
    stride: i32,
    format: PixelFormat,
    backbuffered: bool,
}

impl FramebufferDevice {
    pub fn new(info: &FramebufferInfo) -> Self {
        let draw_len = (info.stride as usize)
            .saturating_mul(info.height as usize)
            .saturating_mul(4);
        let backbuffered = draw_len <= MAX_BACKBUFFER_BYTES;
        let draw_base = if backbuffered {
            core::ptr::addr_of_mut!(BACKBUFFER) as *mut u8
        } else {
            info.base as *mut u8
        };
        Self {
            base: info.base as *mut u8,
            draw_base,
            draw_len,
            width: info.width as i32,
            height: info.height as i32,
            stride: info.stride as i32,
            format: info.pixel_format,
            backbuffered,
        }
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
        self.backbuffered
    }

    pub fn draw_bytes_len(&self) -> usize {
        self.draw_len
    }

    pub fn present_all(&mut self) {
        self.present_rect(self.bounds());
    }

    pub fn present_rect(&mut self, rect: Rect) {
        if !self.backbuffered {
            return;
        }
        let clipped = rect.intersect(self.bounds());
        if clipped.is_empty() {
            return;
        }
        let row_pixels = clipped.width as usize;
        for y in clipped.y..clipped.bottom() {
            let offset = ((y * self.stride + clipped.x) * 4) as usize;
            unsafe {
                let src = self.draw_base.add(offset);
                let dst = self.base.add(offset);
                for pixel in 0..row_pixels {
                    let byte_offset = pixel * 4;
                    let packed = core::ptr::read_unaligned(src.add(byte_offset) as *const u32);
                    (dst.add(byte_offset) as *mut u32).write_volatile(packed);
                }
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
        let mut x0 = start.x;
        let mut y0 = start.y;
        let x1 = end.x;
        let y1 = end.y;
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let radius = (thickness.max(1) - 1) / 2;

        loop {
            self.overlay_fill_rect(
                Rect::new(x0 - radius, y0 - radius, thickness.max(1), thickness.max(1))
                    .intersect(clip),
                color,
            );
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = err * 2;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn overlay_fill_rect(&mut self, rect: Rect, color: Color) {
        let clipped = rect.intersect(self.bounds());
        if clipped.is_empty() {
            return;
        }
        for y in clipped.y..clipped.bottom() {
            let mut offset = ((y * self.stride + clipped.x) * 4) as usize;
            for _ in clipped.x..clipped.right() {
                self.put_physical_pixel_at_offset(offset, color);
                offset += 4;
            }
        }
    }

    pub fn blit_pixels(&mut self, dest: Point, size: super::Size, pixels: &[Color]) {
        if size.width <= 0 || size.height <= 0 {
            return;
        }
        let rect = Rect::new(dest.x, dest.y, size.width, size.height).intersect(self.bounds());
        if rect.is_empty() {
            return;
        }
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let src_x = x - dest.x;
                let src_y = y - dest.y;
                let index = (src_y * size.width + src_x) as usize;
                if let Some(color) = pixels.get(index).copied() {
                    self.put_pixel_unchecked(x, y, color);
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
        let clipped = rect.intersect(bounds);
        self.fill_rect(clipped, Color::rgb(40, 42, 40));

        let mut x = 0;
        while x < bounds.width {
            self.fill_rect(
                Rect::new(x, 0, 1, bounds.height).intersect(clipped),
                Color::rgb(46, 48, 46),
            );
            x += 64;
        }
        let mut y = 0;
        while y < bounds.height {
            self.fill_rect(
                Rect::new(0, y, bounds.width, 1).intersect(clipped),
                Color::rgb(46, 48, 46),
            );
            y += 64;
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
        let clipped = rect.intersect(self.bounds());
        if clipped.is_empty() {
            return;
        }
        for y in clipped.y..clipped.bottom() {
            let mut offset = ((y * self.stride + clipped.x) * 4) as usize;
            for _ in clipped.x..clipped.right() {
                self.put_pixel_at_offset_bytes(offset, color);
                offset += 4;
            }
        }
    }

    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        self.fill_rect(Rect::new(rect.x, rect.y, rect.width, 1), color);
        self.fill_rect(Rect::new(rect.x, rect.bottom() - 1, rect.width, 1), color);
        self.fill_rect(Rect::new(rect.x, rect.y, 1, rect.height), color);
        self.fill_rect(Rect::new(rect.right() - 1, rect.y, 1, rect.height), color);
    }

    pub fn fill_rect_checker(&mut self, rect: Rect, primary: Color, secondary: Color, cell: i32) {
        let clipped = rect.intersect(self.bounds());
        let cell = cell.max(1);
        for y in clipped.y..clipped.bottom() {
            for x in clipped.x..clipped.right() {
                let color = if ((x / cell) + (y / cell)) % 2 == 0 {
                    primary
                } else {
                    secondary
                };
                self.put_pixel_unchecked(x, y, color);
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
        let clipped = rect.intersect(self.bounds());
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
                let color = if ((x / cell) + (y / cell)) % 2 == 0 {
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
        let r2 = radius * radius;
        let rect = Rect::new(
            center.x - radius,
            center.y - radius,
            radius * 2 + 1,
            radius * 2 + 1,
        )
        .intersect(self.bounds());
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let dx = x - center.x;
                let dy = y - center.y;
                if dx * dx + dy * dy <= r2 {
                    self.put_pixel_unchecked(x, y, color);
                }
            }
        }
    }

    pub fn line(&mut self, start: Point, end: Point, thickness: i32, color: Color, clip: Rect) {
        let mut x0 = start.x;
        let mut y0 = start.y;
        let x1 = end.x;
        let y1 = end.y;
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let radius = (thickness.max(1) - 1) / 2;

        loop {
            self.fill_rect(
                Rect::new(x0 - radius, y0 - radius, thickness.max(1), thickness.max(1))
                    .intersect(clip),
                color,
            );
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = err * 2;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn put_pixel_unchecked(&mut self, x: i32, y: i32, color: Color) {
        let offset = ((y * self.stride + x) * 4) as usize;
        self.put_pixel_at_offset_bytes(offset, color);
    }

    fn put_pixel_at_offset_bytes(&mut self, offset: usize, color: Color) {
        unsafe {
            let pixel = self.draw_base.add(offset);
            match self.format {
                PixelFormat::Rgb => {
                    self.write_draw_byte(pixel.add(0), color.r);
                    self.write_draw_byte(pixel.add(1), color.g);
                    self.write_draw_byte(pixel.add(2), color.b);
                }
                _ => {
                    self.write_draw_byte(pixel.add(0), color.b);
                    self.write_draw_byte(pixel.add(1), color.g);
                    self.write_draw_byte(pixel.add(2), color.r);
                }
            }
            self.write_draw_byte(pixel.add(3), 0);
        }
    }

    unsafe fn write_draw_byte(&self, ptr: *mut u8, value: u8) {
        if self.backbuffered {
            ptr.write(value);
        } else {
            ptr.write_volatile(value);
        }
    }

    fn put_physical_pixel_at_offset(&mut self, offset: usize, color: Color) {
        unsafe {
            let pixel = self.base.add(offset);
            match self.format {
                PixelFormat::Rgb => {
                    pixel.add(0).write_volatile(color.r);
                    pixel.add(1).write_volatile(color.g);
                    pixel.add(2).write_volatile(color.b);
                }
                _ => {
                    pixel.add(0).write_volatile(color.b);
                    pixel.add(1).write_volatile(color.g);
                    pixel.add(2).write_volatile(color.r);
                }
            }
            pixel.add(3).write_volatile(0);
        }
    }
}

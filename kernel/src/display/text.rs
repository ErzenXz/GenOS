use super::{stroke_font, Color, FramebufferDevice, Point, Rect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextWeight {
    Regular,
    Bold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextStyle {
    pub size: i32,
    pub color: Color,
    pub weight: TextWeight,
}

impl TextStyle {
    pub const fn regular(size: i32, color: Color) -> Self {
        Self {
            size,
            color,
            weight: TextWeight::Regular,
        }
    }

    pub const fn bold(size: i32, color: Color) -> Self {
        Self {
            size,
            color,
            weight: TextWeight::Bold,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextMetrics {
    pub cell_width: i32,
    pub line_height: i32,
    pub glyph_height: i32,
    pub stroke: i32,
}

pub struct TextRenderer;

impl TextRenderer {
    pub const fn metrics(style: TextStyle) -> TextMetrics {
        let size = if style.size < 10 { 10 } else { style.size };
        let scale = if size >= 18 { 2 } else { 1 };
        let stroke = match style.weight {
            TextWeight::Regular => scale,
            TextWeight::Bold => scale + 1,
        };
        TextMetrics {
            cell_width: size / 2 + 4,
            line_height: size + 6,
            glyph_height: size,
            stroke,
        }
    }

    pub fn draw_text(
        fb: &mut FramebufferDevice,
        clip: Rect,
        origin: Point,
        text: &str,
        style: TextStyle,
    ) -> i32 {
        let metrics = Self::metrics(style);
        let mut x = origin.x;
        for ch in text.chars() {
            if x >= clip.right() {
                break;
            }
            Self::draw_char(fb, clip, Point::new(x, origin.y), ch, style);
            x += metrics.cell_width;
        }
        x - origin.x
    }

    pub fn draw_cursor(fb: &mut FramebufferDevice, clip: Rect, at: Point, style: TextStyle) {
        let metrics = Self::metrics(style);
        let cursor = Rect::new(at.x, at.y + 2, 2, metrics.glyph_height);
        fb.fill_rect(cursor.intersect(clip), Color::ACCENT);
    }

    pub fn draw_char(
        fb: &mut FramebufferDevice,
        clip: Rect,
        origin: Point,
        ch: char,
        style: TextStyle,
    ) {
        if ch == ' ' {
            return;
        }
        let metrics = Self::metrics(style);
        let scale_x = (metrics.cell_width - 2).max(6) / 6;
        let scale_y = metrics.glyph_height.max(9) / 9;
        let scale = scale_x.min(scale_y).max(1);
        let x_pad = (metrics.cell_width - 6 * scale) / 2;
        let y_pad = (metrics.glyph_height - 8 * scale) / 2;

        let glyph = stroke_font::glyph(ch);
        for segment in glyph.segments() {
            let start = Point::new(
                origin.x + x_pad + segment.start.x * scale,
                origin.y + y_pad + segment.start.y * scale,
            );
            let end = Point::new(
                origin.x + x_pad + segment.end.x * scale,
                origin.y + y_pad + segment.end.y * scale,
            );
            fb.line(start, end, metrics.stroke, style.color, clip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_metrics_are_bounded() {
        let metrics = TextRenderer::metrics(TextStyle::regular(16, Color::TEXT));
        assert!(metrics.cell_width >= 10);
        assert!(metrics.line_height >= 20);
        assert!(metrics.stroke >= 1);
    }
}

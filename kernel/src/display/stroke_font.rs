use super::Point;

#[derive(Clone, Copy)]
pub struct Segment {
    pub start: Point,
    pub end: Point,
}

impl Segment {
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self {
            start: Point::new(x0, y0),
            end: Point::new(x1, y1),
        }
    }
}

const MAX_SEGMENTS: usize = 16;
const EMPTY_SEGMENT: Segment = Segment::new(0, 0, 0, 0);

#[derive(Clone, Copy)]
pub struct Glyph {
    segments: [Segment; MAX_SEGMENTS],
    len: usize,
}

impl Glyph {
    pub const fn empty() -> Self {
        Self {
            segments: [EMPTY_SEGMENT; MAX_SEGMENTS],
            len: 0,
        }
    }

    pub fn from_slice(source: &[Segment]) -> Self {
        let mut glyph = Self::empty();
        let mut index = 0;
        while index < source.len() && index < MAX_SEGMENTS {
            glyph.segments[index] = source[index];
            index += 1;
        }
        glyph.len = index;
        glyph
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.len]
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub fn glyph(ch: char) -> Glyph {
    match normalize(ch) {
        'A' => Glyph::from_slice(&[s(0, 8, 3, 0), s(3, 0, 6, 8), s(1, 5, 5, 5)]),
        'B' => Glyph::from_slice(&[
            s(0, 0, 0, 8),
            s(0, 0, 4, 0),
            s(4, 0, 5, 1),
            s(5, 1, 5, 3),
            s(5, 3, 4, 4),
            s(0, 4, 4, 4),
            s(4, 4, 5, 5),
            s(5, 5, 5, 7),
            s(5, 7, 4, 8),
            s(0, 8, 4, 8),
        ]),
        'C' => Glyph::from_slice(&[
            s(6, 1, 5, 0),
            s(5, 0, 1, 0),
            s(1, 0, 0, 1),
            s(0, 1, 0, 7),
            s(0, 7, 1, 8),
            s(1, 8, 5, 8),
            s(5, 8, 6, 7),
        ]),
        'D' => Glyph::from_slice(&[
            s(0, 0, 0, 8),
            s(0, 0, 4, 0),
            s(4, 0, 6, 2),
            s(6, 2, 6, 6),
            s(6, 6, 4, 8),
            s(4, 8, 0, 8),
        ]),
        'E' => Glyph::from_slice(&[s(0, 0, 0, 8), s(0, 0, 6, 0), s(0, 4, 5, 4), s(0, 8, 6, 8)]),
        'F' => Glyph::from_slice(&[s(0, 0, 0, 8), s(0, 0, 6, 0), s(0, 4, 5, 4)]),
        'G' => Glyph::from_slice(&[
            s(6, 1, 5, 0),
            s(5, 0, 1, 0),
            s(1, 0, 0, 1),
            s(0, 1, 0, 7),
            s(0, 7, 1, 8),
            s(1, 8, 5, 8),
            s(5, 8, 6, 7),
            s(6, 7, 6, 5),
            s(6, 5, 3, 5),
        ]),
        'H' => Glyph::from_slice(&[s(0, 0, 0, 8), s(6, 0, 6, 8), s(0, 4, 6, 4)]),
        'I' => Glyph::from_slice(&[s(1, 0, 5, 0), s(3, 0, 3, 8), s(1, 8, 5, 8)]),
        'J' => Glyph::from_slice(&[
            s(1, 0, 6, 0),
            s(5, 0, 5, 7),
            s(5, 7, 4, 8),
            s(4, 8, 1, 8),
            s(1, 8, 0, 7),
        ]),
        'K' => Glyph::from_slice(&[s(0, 0, 0, 8), s(6, 0, 0, 4), s(0, 4, 6, 8)]),
        'L' => Glyph::from_slice(&[s(0, 0, 0, 8), s(0, 8, 6, 8)]),
        'M' => Glyph::from_slice(&[s(0, 8, 0, 0), s(0, 0, 3, 4), s(3, 4, 6, 0), s(6, 0, 6, 8)]),
        'N' => Glyph::from_slice(&[s(0, 8, 0, 0), s(0, 0, 6, 8), s(6, 8, 6, 0)]),
        'O' => Glyph::from_slice(&[
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 7),
            s(6, 7, 5, 8),
            s(5, 8, 1, 8),
            s(1, 8, 0, 7),
            s(0, 7, 0, 1),
            s(0, 1, 1, 0),
        ]),
        'P' => Glyph::from_slice(&[
            s(0, 0, 0, 8),
            s(0, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 3),
            s(6, 3, 5, 4),
            s(5, 4, 0, 4),
        ]),
        'Q' => Glyph::from_slice(&[
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 7),
            s(6, 7, 5, 8),
            s(5, 8, 1, 8),
            s(1, 8, 0, 7),
            s(0, 7, 0, 1),
            s(0, 1, 1, 0),
            s(4, 6, 6, 8),
        ]),
        'R' => Glyph::from_slice(&[
            s(0, 0, 0, 8),
            s(0, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 3),
            s(6, 3, 5, 4),
            s(5, 4, 0, 4),
            s(2, 4, 6, 8),
        ]),
        'S' => Glyph::from_slice(&[
            s(6, 1, 5, 0),
            s(5, 0, 1, 0),
            s(1, 0, 0, 1),
            s(0, 1, 0, 3),
            s(0, 3, 1, 4),
            s(1, 4, 5, 4),
            s(5, 4, 6, 5),
            s(6, 5, 6, 7),
            s(6, 7, 5, 8),
            s(5, 8, 1, 8),
            s(1, 8, 0, 7),
        ]),
        'T' => Glyph::from_slice(&[s(0, 0, 6, 0), s(3, 0, 3, 8)]),
        'U' => Glyph::from_slice(&[
            s(0, 0, 0, 7),
            s(0, 7, 1, 8),
            s(1, 8, 5, 8),
            s(5, 8, 6, 7),
            s(6, 7, 6, 0),
        ]),
        'V' => Glyph::from_slice(&[s(0, 0, 3, 8), s(3, 8, 6, 0)]),
        'W' => Glyph::from_slice(&[s(0, 0, 1, 8), s(1, 8, 3, 4), s(3, 4, 5, 8), s(5, 8, 6, 0)]),
        'X' => Glyph::from_slice(&[s(0, 0, 6, 8), s(6, 0, 0, 8)]),
        'Y' => Glyph::from_slice(&[s(0, 0, 3, 4), s(6, 0, 3, 4), s(3, 4, 3, 8)]),
        'Z' => Glyph::from_slice(&[s(0, 0, 6, 0), s(6, 0, 0, 8), s(0, 8, 6, 8)]),
        '0' => Glyph::from_slice(&[
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 7),
            s(6, 7, 5, 8),
            s(5, 8, 1, 8),
            s(1, 8, 0, 7),
            s(0, 7, 0, 1),
            s(0, 1, 1, 0),
            s(1, 7, 5, 1),
        ]),
        '1' => Glyph::from_slice(&[s(2, 2, 3, 0), s(3, 0, 3, 8), s(1, 8, 5, 8)]),
        '2' => Glyph::from_slice(&[
            s(0, 1, 1, 0),
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 3),
            s(6, 3, 0, 8),
            s(0, 8, 6, 8),
        ]),
        '3' => Glyph::from_slice(&[
            s(0, 1, 1, 0),
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 4, 4),
            s(4, 4, 6, 6),
            s(6, 6, 5, 8),
            s(5, 8, 1, 8),
            s(1, 8, 0, 7),
        ]),
        '4' => Glyph::from_slice(&[s(5, 8, 5, 0), s(0, 5, 6, 5), s(0, 5, 5, 0)]),
        '5' => Glyph::from_slice(&[
            s(6, 0, 0, 0),
            s(0, 0, 0, 4),
            s(0, 4, 5, 4),
            s(5, 4, 6, 5),
            s(6, 5, 6, 7),
            s(6, 7, 5, 8),
            s(5, 8, 0, 8),
        ]),
        '6' => Glyph::from_slice(&[
            s(6, 1, 5, 0),
            s(5, 0, 1, 0),
            s(1, 0, 0, 1),
            s(0, 1, 0, 7),
            s(0, 7, 1, 8),
            s(1, 8, 5, 8),
            s(5, 8, 6, 7),
            s(6, 7, 6, 5),
            s(6, 5, 5, 4),
            s(5, 4, 0, 4),
        ]),
        '7' => Glyph::from_slice(&[s(0, 0, 6, 0), s(6, 0, 2, 8)]),
        '8' => Glyph::from_slice(&[
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 3),
            s(6, 3, 5, 4),
            s(5, 4, 1, 4),
            s(1, 4, 0, 3),
            s(0, 3, 0, 1),
            s(0, 1, 1, 0),
            s(1, 4, 0, 5),
            s(0, 5, 0, 7),
            s(0, 7, 1, 8),
            s(1, 8, 5, 8),
            s(5, 8, 6, 7),
            s(6, 7, 6, 5),
            s(6, 5, 5, 4),
        ]),
        '9' => Glyph::from_slice(&[
            s(6, 4, 1, 4),
            s(1, 4, 0, 3),
            s(0, 3, 0, 1),
            s(0, 1, 1, 0),
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 7),
            s(6, 7, 5, 8),
            s(5, 8, 1, 8),
        ]),
        '-' => Glyph::from_slice(&[s(1, 4, 5, 4)]),
        '_' => Glyph::from_slice(&[s(0, 8, 6, 8)]),
        '=' => Glyph::from_slice(&[s(1, 3, 5, 3), s(1, 6, 5, 6)]),
        '+' => Glyph::from_slice(&[s(3, 2, 3, 6), s(1, 4, 5, 4)]),
        '/' => Glyph::from_slice(&[s(6, 0, 0, 8)]),
        '\\' => Glyph::from_slice(&[s(0, 0, 6, 8)]),
        '>' => Glyph::from_slice(&[s(1, 1, 5, 4), s(5, 4, 1, 7)]),
        '<' => Glyph::from_slice(&[s(5, 1, 1, 4), s(1, 4, 5, 7)]),
        ':' => Glyph::from_slice(&[s(3, 2, 3, 2), s(3, 6, 3, 6)]),
        '.' => Glyph::from_slice(&[s(3, 8, 3, 8)]),
        ',' => Glyph::from_slice(&[s(3, 7, 2, 9)]),
        '\'' => Glyph::from_slice(&[s(3, 0, 2, 2)]),
        '"' => Glyph::from_slice(&[s(2, 0, 2, 2), s(4, 0, 4, 2)]),
        '(' => Glyph::from_slice(&[s(4, 0, 2, 2), s(2, 2, 2, 6), s(2, 6, 4, 8)]),
        ')' => Glyph::from_slice(&[s(2, 0, 4, 2), s(4, 2, 4, 6), s(4, 6, 2, 8)]),
        '[' => Glyph::from_slice(&[s(4, 0, 1, 0), s(1, 0, 1, 8), s(1, 8, 4, 8)]),
        ']' => Glyph::from_slice(&[s(2, 0, 5, 0), s(5, 0, 5, 8), s(5, 8, 2, 8)]),
        '|' => Glyph::from_slice(&[s(3, 0, 3, 8)]),
        '!' => Glyph::from_slice(&[s(3, 0, 3, 6), s(3, 8, 3, 8)]),
        '?' => Glyph::from_slice(&[
            s(0, 1, 1, 0),
            s(1, 0, 5, 0),
            s(5, 0, 6, 1),
            s(6, 1, 6, 3),
            s(6, 3, 3, 5),
            s(3, 5, 3, 6),
            s(3, 8, 3, 8),
        ]),
        _ => Glyph::empty(),
    }
}

#[cfg(test)]
pub fn has_glyph(ch: char) -> bool {
    ch == ' ' || !glyph(ch).is_empty()
}

const fn s(x0: i32, y0: i32, x1: i32, y1: i32) -> Segment {
    Segment::new(x0, y0, x1, y1)
}

fn normalize(ch: char) -> char {
    if ch.is_ascii_lowercase() {
        (ch as u8 - b'a' + b'A') as char
    } else {
        ch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_shell_glyphs_exist() {
        for ch in "GenOS help clear mem ls cat README.TXT >:/=-_0123456789".chars() {
            assert!(has_glyph(ch), "missing glyph {ch}");
        }
    }
}

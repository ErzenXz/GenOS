#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

impl Size {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(self) -> i32 {
        self.x + self.width
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.height
    }

    pub const fn is_empty(self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn inset(self, amount: i32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            self.width - amount * 2,
            self.height - amount * 2,
        )
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    pub fn intersect(self, other: Self) -> Self {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());
        Self::new(x1, y1, (x2 - x1).max(0), (y2 - y1).max(0))
    }

    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = self.right().max(other.right());
        let y2 = self.bottom().max(other.bottom());
        Self::new(x1, y1, x2 - x1, y2 - y1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_rectangles() {
        let a = Rect::new(10, 10, 40, 30);
        let b = Rect::new(30, 0, 30, 30);
        assert_eq!(a.intersect(b), Rect::new(30, 10, 20, 20));
    }

    #[test]
    fn non_overlapping_clip_is_empty() {
        assert!(Rect::new(0, 0, 10, 10)
            .intersect(Rect::new(20, 20, 5, 5))
            .is_empty());
    }

    #[test]
    fn unions_rectangles() {
        assert_eq!(
            Rect::new(10, 10, 20, 20).union(Rect::new(25, 5, 10, 10)),
            Rect::new(10, 5, 25, 25)
        );
    }
}

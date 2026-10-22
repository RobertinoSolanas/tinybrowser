//! Geometry value objects: `Point`, `Size`, `Rect`, `Sides`.
//!
//! All coordinates are f32 device independent pixels (1 css px == 1 unit).

use std::fmt;

/// A 2D point.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

/// A 2D size.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };
    pub const fn new(width: f32, height: f32) -> Self {
        Size { width, height }
    }
}

/// An axis aligned rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// Rectangle from two opposite corners.
    pub fn from_points(a: Point, b: Point) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        Rect {
            x,
            y,
            width: (b.x - a.x).abs(),
            height: (b.y - a.y).abs(),
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
    pub fn left(&self) -> f32 {
        self.x
    }
    pub fn top(&self) -> f32 {
        self.y
    }
    pub fn area(&self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Shrink by the given amounts on each side.
    pub fn inset(&self, top: f32, right: f32, bottom: f32, left: f32) -> Rect {
        Rect {
            x: self.x + left,
            y: self.y + top,
            width: (self.width - left - right).max(0.0),
            height: (self.height - top - bottom).max(0.0),
        }
    }

    /// Grow by the given amounts on each side.
    pub fn outset(&self, top: f32, right: f32, bottom: f32, left: f32) -> Rect {
        self.inset(-top, -right, -bottom, -left)
    }

    pub fn translated(&self, dx: f32, dy: f32) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }

    /// Intersection (may be empty).
    pub fn intersect(&self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Rect {
            x,
            y,
            width: (self.right().min(other.right()) - x).max(0.0),
            height: (self.bottom().min(other.bottom()) - y).max(0.0),
        }
    }

    /// Round to whole pixels (used by the rasterizer).
    pub fn rounded(&self) -> Rect {
        let x = self.x.round();
        let y = self.y.round();
        Rect {
            x,
            y,
            width: (self.right() - x).round(),
            height: (self.bottom() - y).round(),
        }
    }
}

/// The four physical sides of a box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sides {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Sides {
    pub const ZERO: Sides = Sides {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };
    pub const fn all(v: f32) -> Sides {
        Sides {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }
    pub const fn xy(h: f32, v: f32) -> Sides {
        Sides {
            top: v,
            right: h,
            bottom: v,
            left: h,
        }
    }
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({:.1},{:.1} {:.1}x{:.1})",
            self.x, self.y, self.width, self.height
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(r.contains(Point::new(10.0, 10.0)));
        assert!(r.contains(Point::new(29.9, 29.9)));
        assert!(!r.contains(Point::new(30.0, 15.0)));
    }

    #[test]
    fn inset_never_inverts() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(2.0, 30.0, 2.0, 30.0);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 6.0);
    }

    #[test]
    fn union_and_intersect() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.union(b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert_eq!(a.intersect(b), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(a.intersect(Rect::new(100.0, 100.0, 1.0, 1.0)).is_empty());
    }

    #[test]
    fn sides_helpers() {
        let s = Sides::xy(4.0, 8.0);
        assert_eq!(s.horizontal(), 8.0);
        assert_eq!(s.vertical(), 16.0);
    }
}

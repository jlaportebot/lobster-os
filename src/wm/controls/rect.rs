//! Signed rectangle used for control layout inside a scrolled view.

/// An axis-aligned rectangle in view coordinates. The origin is signed
/// because content scrolled above a viewport has a negative `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in px.
    pub w: u32,
    /// Height in px.
    pub h: u32,
}

impl Rect {
    /// Rectangle from its origin and size.
    #[must_use]
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// First column right of the rectangle.
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    /// First row below the rectangle.
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    /// Whether the point lies inside.
    #[must_use]
    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Overlap of the two rectangles, `None` when they do not touch.
    #[must_use]
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
    }

    /// The rectangle moved by (`dx`, `dy`).
    #[must_use]
    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The rectangle shrunk by `n` px on every side.
    #[must_use]
    pub const fn inset(&self, n: u32) -> Rect {
        let n2 = 2 * n;
        let (w, h) = (self.w.saturating_sub(n2), self.h.saturating_sub(n2));
        Rect::new(self.x + n as i32, self.y + n as i32, w, h)
    }
}

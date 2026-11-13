//! Geometry in local logical pixels. Paths are shared, copy-on-write values.
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub const ZERO: Self = Self::new(0.0, 0.0);
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}
impl From<(f64, f64)> for Point {
    fn from((x, y): (f64, f64)) -> Self {
        Self::new(x, y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}
impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn size(self) -> Size {
        Size::new(self.width, self.height)
    }
    pub fn center(self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
    pub fn top_center(self) -> Point {
        Point::new(self.center().x, self.y)
    }
    pub fn bottom_left(self) -> Point {
        Point::new(self.x, self.y + self.height)
    }
    pub fn bottom_right(self) -> Point {
        Point::new(self.x + self.width, self.y + self.height)
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.width && p.y < self.y + self.height
    }
    pub fn inflate(self, amount: f64) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.width + amount * 2.0,
            self.height + amount * 2.0,
        )
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            (self.x + self.width).max(other.x + other.width) - x,
            (self.y + self.height).max(other.y + other.height) - y,
        )
    }
    pub fn transformed(self, matrix: crate::Affine2d) -> Self {
        let [a, b, c, d, e, f] = matrix.0;
        let points = [
            (self.x, self.y),
            (self.x + self.width, self.y),
            (self.x, self.y + self.height),
            (self.x + self.width, self.y + self.height),
        ]
        .map(|(x, y)| Point::new(a * x + c * y + e, b * x + d * y + f));
        bounds(points).unwrap_or_default()
    }
    pub(crate) fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width >= 0.0
            && self.height >= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathElement {
    MoveTo(Point),
    LineTo(Point),
    QuadTo(Point, Point),
    CubicTo(Point, Point, Point),
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path(Arc<Vec<PathElement>>);
impl Path {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn move_to(&mut self, point: impl Into<Point>) {
        Arc::make_mut(&mut self.0).push(PathElement::MoveTo(point.into()));
    }
    pub fn line_to(&mut self, point: impl Into<Point>) {
        Arc::make_mut(&mut self.0).push(PathElement::LineTo(point.into()));
    }
    pub fn quad_to(&mut self, control: impl Into<Point>, end: impl Into<Point>) {
        Arc::make_mut(&mut self.0).push(PathElement::QuadTo(control.into(), end.into()));
    }
    pub fn cubic_to(
        &mut self,
        first: impl Into<Point>,
        second: impl Into<Point>,
        end: impl Into<Point>,
    ) {
        Arc::make_mut(&mut self.0).push(PathElement::CubicTo(
            first.into(),
            second.into(),
            end.into(),
        ));
    }
    pub fn close(&mut self) {
        if !self.0.is_empty() && self.0.last() != Some(&PathElement::Close) {
            Arc::make_mut(&mut self.0).push(PathElement::Close);
        }
    }
    pub fn elements(&self) -> &[PathElement] {
        &self.0
    }
    pub fn polyline(points: impl IntoIterator<Item = Point>) -> Self {
        let mut result = Self::new();
        for (index, point) in points.into_iter().enumerate() {
            if index == 0 {
                result.move_to(point);
            } else {
                result.line_to(point);
            }
        }
        result
    }
    pub fn polygon(points: impl IntoIterator<Item = Point>) -> Self {
        let mut result = Self::polyline(points);
        result.close();
        result
    }
    pub fn rectangle(r: Rect) -> Self {
        Self::polygon([
            Point::new(r.x, r.y),
            Point::new(r.x + r.width, r.y),
            r.bottom_right(),
            r.bottom_left(),
        ])
    }
    pub fn ellipse(r: Rect) -> Self {
        let mut path = Self::arc(
            r.center(),
            Size::new(r.width / 2.0, r.height / 2.0),
            0.0,
            std::f64::consts::TAU,
        );
        path.close();
        path
    }
    /// Elliptical arc. Angles are radians, clockwise with y increasing downward.
    /// Sweeps are limited to one revolution; invalid values fail validation.
    pub fn arc(center: Point, radii: Size, start: f64, sweep: f64) -> Self {
        let mut path = Self::new();
        if !start.is_finite() || !sweep.is_finite() {
            path.move_to((f64::NAN, f64::NAN));
            return path;
        }
        let sweep = sweep.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
        let count = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
        let step = sweep / count as f64;
        let point = |t: f64| {
            Point::new(
                center.x + radii.width * t.cos(),
                center.y + radii.height * t.sin(),
            )
        };
        path.move_to(point(start));
        for i in 0..count {
            let a = start + step * i as f64;
            let b = a + step;
            let k = 4.0 / 3.0 * (step / 4.0).tan();
            let p = point(a);
            let q = point(b);
            path.cubic_to(
                Point::new(
                    p.x - k * radii.width * a.sin(),
                    p.y + k * radii.height * a.cos(),
                ),
                Point::new(
                    q.x + k * radii.width * b.sin(),
                    q.y - k * radii.height * b.cos(),
                ),
                q,
            );
        }
        path
    }
    /// Conservative control-point bounds, including all Bézier coverage.
    pub fn bounds(&self) -> Option<Rect> {
        bounds(self.points())
    }
    fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.0.iter().flat_map(|el| {
            match *el {
                PathElement::MoveTo(a) | PathElement::LineTo(a) => [Some(a), None, None],
                PathElement::QuadTo(a, b) => [Some(a), Some(b), None],
                PathElement::CubicTo(a, b, c) => [Some(a), Some(b), Some(c)],
                PathElement::Close => [None, None, None],
            }
            .into_iter()
            .flatten()
        })
    }
    pub(crate) fn valid(&self) -> bool {
        self.points().all(|p| p.x.is_finite() && p.y.is_finite())
    }
}
fn bounds(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
    let mut points = points.into_iter();
    let first = points.next()?;
    let (mut x, mut y, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
    for p in points {
        x = x.min(p.x);
        y = y.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    Some(Rect::new(x, y, right - x, bottom - y))
}

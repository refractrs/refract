//! Shared border-box geometry for fills, borders, clipping, and shadows.

use super::Rect;
use refract_core::{Border, CornerRadii, Insets};
use vello::{
    Scene,
    kurbo::{Affine, BezPath, Rect as KRect},
    peniko::{Color, Fill},
};

#[derive(Clone, Copy, Debug)]
pub(super) struct RoundedBox {
    pub bounds: KRect,
    // Clockwise from top-left. Inner corners can be elliptical.
    pub corners: [(f64, f64); 4],
}

impl RoundedBox {
    pub fn new(rect: Rect, radii: CornerRadii) -> Self {
        let width = f64::from(rect.width.max(0.0));
        let height = f64::from(rect.height.max(0.0));
        let r = [
            radii.top_left,
            radii.top_right,
            radii.bottom_right,
            radii.bottom_left,
        ]
        .map(|r| f64::from(r.max(0.0)));
        // CSS scales all corners by the same factor when adjacent radii overlap.
        let mut factor = 1.0_f64;
        for (size, sum) in [
            (width, r[0] + r[1]),
            (width, r[3] + r[2]),
            (height, r[0] + r[3]),
            (height, r[1] + r[2]),
        ] {
            if sum > 0.0 {
                factor = factor.min(size / sum);
            }
        }
        Self {
            bounds: KRect::new(
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.x) + width,
                f64::from(rect.y) + height,
            ),
            corners: r.map(|r| (r * factor, r * factor)),
        }
    }

    pub fn inset(self, insets: Insets) -> Self {
        let b = self.bounds;
        let (mut l, mut r, mut t, mut d) = (
            f64::from(insets.left),
            f64::from(insets.right),
            f64::from(insets.top),
            f64::from(insets.bottom),
        );
        // Oversized opposing borders meet without crossing.
        let sx = if l + r > 0.0 {
            (b.width() / (l + r)).min(1.0)
        } else {
            1.0
        };
        let sy = if t + d > 0.0 {
            (b.height() / (t + d)).min(1.0)
        } else {
            1.0
        };
        l *= sx;
        r *= sx;
        t *= sy;
        d *= sy;
        let mut corners = self.corners;
        for (corner, (x, y)) in corners.iter_mut().zip([(l, t), (r, t), (r, d), (l, d)]) {
            *corner = ((corner.0 - x).max(0.0), (corner.1 - y).max(0.0));
        }
        let bounds = KRect::new(b.x0 + l, b.y0 + t, b.x1 - r, b.y1 - d);
        // Large asymmetric insets can leave corner ellipses wider than the
        // remaining content box. Keep that path inside its clip rectangle.
        let mut factor = 1.0_f64;
        for (size, sum) in [
            (bounds.width(), corners[0].0 + corners[1].0),
            (bounds.width(), corners[3].0 + corners[2].0),
            (bounds.height(), corners[0].1 + corners[3].1),
            (bounds.height(), corners[1].1 + corners[2].1),
        ] {
            if sum > 0.0 {
                factor = factor.min(size.max(0.0) / sum);
            }
        }
        Self {
            bounds,
            corners: corners.map(|(x, y)| (x * factor, y * factor)),
        }
    }

    pub fn path(self) -> BezPath {
        let b = self.bounds;
        if b.width() <= 0.0 || b.height() <= 0.0 {
            return BezPath::new();
        }
        let [(a, e), (c, f), (d, g), (h, i)] = self.corners;
        // Cubic approximation of a quarter ellipse (error below 0.03%).
        const K: f64 = 0.552_284_749_830_793_6;
        let mut p = BezPath::new();
        p.move_to((b.x0 + a, b.y0));
        p.line_to((b.x1 - c, b.y0));
        p.curve_to(
            (b.x1 - c + K * c, b.y0),
            (b.x1, b.y0 + f - K * f),
            (b.x1, b.y0 + f),
        );
        p.line_to((b.x1, b.y1 - g));
        p.curve_to(
            (b.x1, b.y1 - g + K * g),
            (b.x1 - d + K * d, b.y1),
            (b.x1 - d, b.y1),
        );
        p.line_to((b.x0 + h, b.y1));
        p.curve_to(
            (b.x0 + h - K * h, b.y1),
            (b.x0, b.y1 - i + K * i),
            (b.x0, b.y1 - i),
        );
        p.line_to((b.x0, b.y0 + e));
        p.curve_to(
            (b.x0, b.y0 + e - K * e),
            (b.x0 + a - K * a, b.y0),
            (b.x0 + a, b.y0),
        );
        p.close_path();
        p
    }

    pub fn contains(self, x: f64, y: f64) -> bool {
        let b = self.bounds;
        if x < b.x0 || x > b.x1 || y < b.y0 || y > b.y1 {
            return false;
        }
        for ((rx, ry), (dx, dy)) in self.corners.into_iter().zip([
            (x - b.x0, y - b.y0),
            (b.x1 - x, y - b.y0),
            (b.x1 - x, b.y1 - y),
            (x - b.x0, b.y1 - y),
        ]) {
            if rx > 0.0
                && ry > 0.0
                && dx < rx
                && dy < ry
                && ((rx - dx) / rx).powi(2) + ((ry - dy) / ry).powi(2) > 1.0
            {
                return false;
            }
        }
        true
    }
}

pub(super) fn rounded(rect: Rect, radii: CornerRadii) -> BezPath {
    RoundedBox::new(rect, radii).path()
}

pub(super) fn border(
    scene: &mut Scene,
    transform: Affine,
    rect: Rect,
    radii: CornerRadii,
    border: Border,
) {
    let outer = RoundedBox::new(rect, radii);
    let inner = outer.inset(Insets {
        top: border.top.width,
        right: border.right.width,
        bottom: border.bottom.width,
        left: border.left.width,
    });
    let mut ring = outer.path();
    ring.extend(inner.path().elements().iter().copied());
    let sides = [border.top, border.right, border.bottom, border.left];
    let color = |c: refract_core::Color| Color::from_rgba8(c.0, c.1, c.2, c.3);
    if sides.iter().all(|s| s.color == sides[0].color) {
        scene.fill(Fill::EvenOdd, transform, color(sides[0].color), None, &ring);
        return;
    }
    let points = |b: KRect| [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)];
    let outside = points(outer.bounds);
    let inside = points(inner.bounds);
    scene.push_clip_layer(Fill::EvenOdd, transform, &ring);
    for (i, side) in sides.iter().enumerate() {
        if side.width == 0.0 || side.color.3 == 0 {
            continue;
        }
        let next = (i + 1) % 4;
        let mut wedge = BezPath::new();
        wedge.move_to(outside[i]);
        wedge.line_to(outside[next]);
        wedge.line_to(inside[next]);
        wedge.line_to(inside[i]);
        wedge.close_path();
        scene.fill(Fill::NonZero, transform, color(side.color), None, &wedge);
    }
    scene.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello::kurbo::Shape;

    #[test]
    fn corners_scale_together_and_insets_create_ellipses() {
        let rect = Rect {
            width: 120.0,
            height: 80.0,
            ..Rect::default()
        };
        let shape = RoundedBox::new(rect, CornerRadii::default().top_left(100.0).top_right(50.0));
        assert_eq!(
            shape.corners,
            [(80.0, 80.0), (40.0, 40.0), (0.0, 0.0), (0.0, 0.0)]
        );
        let inner = shape.inset(Insets {
            left: 20.0,
            top: 5.0,
            ..Insets::default()
        });
        assert_eq!(inner.corners[0], (60.0, 75.0));
        assert!(!shape.contains(0.0, 0.0));
        assert!(shape.contains(1.0, 79.0));
        assert_eq!(shape.path().winding((1.0, 79.0).into()), 1);
        assert_eq!(shape.path().winding((0.0, 0.0).into()), 0);
    }

    #[test]
    fn oversized_borders_collapse_without_inverting_inner_box() {
        let shape = RoundedBox::new(
            Rect {
                width: 30.0,
                height: 10.0,
                ..Rect::default()
            },
            20.0.into(),
        );
        let inner = shape.inset(Insets::all(100.0));
        assert_eq!(inner.bounds.width(), 0.0);
        assert_eq!(inner.bounds.height(), 0.0);
        assert_eq!(inner.path().area(), 0.0);

        let skinny = shape.inset(Insets {
            left: 29.0,
            ..Insets::default()
        });
        assert_eq!(skinny.path().bounding_box(), skinny.bounds);
    }
}

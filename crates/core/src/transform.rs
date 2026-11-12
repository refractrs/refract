//! Two-dimensional presentation geometry, independent of layout.
use crate::ViewError;

/// Normalized transform origin. Values outside the box are allowed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPoint {
    pub x: f32,
    pub y: f32,
}
impl UnitPoint {
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    pub const TOP_LEADING: Self = Self { x: 0.0, y: 0.0 };
}
impl Default for UnitPoint {
    fn default() -> Self {
        Self::CENTER
    }
}

/// Applied as translation × rotation × scale about `anchor`. Nested elements
/// compose outside-in; transforms never change a parent's layout allocation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
    pub anchor: UnitPoint,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            anchor: UnitPoint::CENTER,
        }
    }
}
impl Transform {
    pub fn validate(self) -> Result<(), ViewError> {
        if [
            self.offset_x,
            self.offset_y,
            self.scale_x,
            self.scale_y,
            self.rotation,
            self.anchor.x,
            self.anchor.y,
        ]
        .iter()
        .all(|x| x.is_finite())
        {
            Ok(())
        } else {
            Err(ViewError::new("transform values must be finite"))
        }
    }
    pub fn matrix(self, x: f32, y: f32, width: f32, height: f32) -> Affine2d {
        let ax = f64::from(x + width * self.anchor.x);
        let ay = f64::from(y + height * self.anchor.y);
        let (s, c) = f64::from(self.rotation).to_radians().sin_cos();
        let a = c * f64::from(self.scale_x);
        let b = s * f64::from(self.scale_x);
        let c = -s * f64::from(self.scale_y);
        let d = f64::from(self.rotation).to_radians().cos() * f64::from(self.scale_y);
        Affine2d([
            a,
            b,
            c,
            d,
            ax + f64::from(self.offset_x) - a * ax - c * ay,
            ay + f64::from(self.offset_y) - b * ax - d * ay,
        ])
    }
}

/// Column-vector affine matrix in CSS/kurbo coefficient order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2d(pub [f64; 6]);
impl Default for Affine2d {
    fn default() -> Self {
        Self::IDENTITY
    }
}
impl Affine2d {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    pub fn then(self, inner: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = inner.0;
        Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }
    pub fn point(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (
            (a * f64::from(x) + c * f64::from(y) + e) as f32,
            (b * f64::from(x) + d * f64::from(y) + f) as f32,
        )
    }
    pub fn inverse(self) -> Option<Self> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        Some(Self([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_transforms_round_trip_and_preserve_anchor() {
        let a = Transform {
            scale_x: 2.0,
            scale_y: 0.5,
            rotation: 32.0,
            ..Default::default()
        }
        .matrix(10.0, 20.0, 100.0, 80.0);
        let (x, y) = a.point(60.0, 60.0);
        assert!((x - 60.0).abs() < 0.001 && (y - 60.0).abs() < 0.001);
        let b = Transform {
            offset_x: 20.0,
            ..Default::default()
        }
        .matrix(0.0, 0.0, 20.0, 20.0);
        let m = a.then(b);
        let (x, y) = m.point(5.0, 7.0);
        let (x, y) = m.inverse().unwrap().point(x, y);
        assert!((x - 5.0).abs() < 0.001 && (y - 7.0).abs() < 0.001);
        assert!(Transform {
            scale_x: 0.0,
            ..Default::default()
        }
        .matrix(0.0, 0.0, 1.0, 1.0)
        .inverse()
        .is_none());
    }
}

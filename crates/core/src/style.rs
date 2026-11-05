//! Portable style values and validation shared by host adapters.

use crate::ViewError;

/// RGBA color with eight-bit channels, including alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    pub const WHITE: Self = Self(255, 255, 255, 255);
    pub const BLACK: Self = Self(0, 0, 0, 255);
    pub const BLUE: Self = Self(45, 87, 232, 255);

    /// Construct an opaque color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }

    /// Encode this color for adapters that accept CSS color values.
    pub fn css(self) -> String {
        format!(
            "rgba({},{},{},{})",
            self.0,
            self.1,
            self.2,
            self.3 as f32 / 255.0
        )
    }
}

/// A color and its position along a gradient line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// Position along the gradient line, from zero to one (inclusive).
    pub offset: f32,
    pub color: Color,
}

impl GradientStop {
    pub const fn new(offset: f32, color: Color) -> Self {
        Self { offset, color }
    }
}

/// A non-repeating linear gradient spanning an element's border box.
/// Hosts interpolate in premultiplied sRGB and clamp beyond the first/last stop.
/// Angles are clockwise from up: 90° points right, 180° points down.
/// Stops must be ordered, finite, and in 0..=1; equal offsets form hard edges.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub angle: f32,
    pub stops: Vec<GradientStop>,
}

impl LinearGradient {
    /// Construct a two-color gradient with stops at zero and one.
    pub fn new(angle: f32, start: Color, end: Color) -> Self {
        Self {
            angle,
            stops: vec![GradientStop::new(0.0, start), GradientStop::new(1.0, end)],
        }
    }

    pub fn validate(&self) -> Result<(), ViewError> {
        if !self.angle.is_finite() || self.stops.len() < 2 {
            return Err(ViewError::new(
                "gradients need a finite angle and at least two stops",
            ));
        }
        if self
            .stops
            .iter()
            .any(|s| !s.offset.is_finite() || !(0.0..=1.0).contains(&s.offset))
            || self.stops.windows(2).any(|s| s[0].offset > s[1].offset)
        {
            return Err(ViewError::new(
                "gradient stops must be ordered and within zero to one",
            ));
        }
        Ok(())
    }

    /// Border-box-relative endpoints, using CSS's projected gradient-line
    /// length. A diagonal line can extend outside a non-square box.
    pub fn endpoints(&self, width: f64, height: f64) -> ((f64, f64), (f64, f64)) {
        let angle = (self.angle as f64).rem_euclid(360.0).to_radians();
        let (x, y) = (angle.sin(), -angle.cos());
        let half = (width * x.abs() + height * y.abs()) / 2.0;
        (
            (width / 2.0 - x * half, height / 2.0 - y * half),
            (width / 2.0 + x * half, height / 2.0 + y * half),
        )
    }
}

/// A single outer shadow following the element's rounded border box.
/// Offsets and blur radius use logical pixels. Like CSS `box-shadow`, blur is
/// twice the Gaussian standard deviation. Shadows do not affect layout, hit
/// testing, or scroll extents. Inset shadows and spread are not supported yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: Color,
}

impl BoxShadow {
    pub const fn new(offset_x: f32, offset_y: f32, blur_radius: f32, color: Color) -> Self {
        Self {
            offset_x,
            offset_y,
            blur_radius,
            color,
        }
    }

    pub fn validate(&self) -> Result<(), ViewError> {
        if !self.offset_x.is_finite()
            || !self.offset_y.is_finite()
            || !self.blur_radius.is_finite()
            || self.blur_radius < 0.0
        {
            return Err(ViewError::new(
                "shadow offsets must be finite and blur must be finite and nonnegative",
            ));
        }
        Ok(())
    }
}

/// Padding on each edge, measured in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const fn all(n: f32) -> Self {
        Self {
            top: n,
            right: n,
            bottom: n,
            left: n,
        }
    }

    /// Apply horizontal padding to left/right and vertical padding to top/bottom.
    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }
}

impl From<f32> for Insets {
    fn from(n: f32) -> Self {
        Self::all(n)
    }
}

/// One solid border edge, measured in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSide {
    pub width: f32,
    pub color: Color,
}

impl BorderSide {
    pub const fn new(width: f32, color: Color) -> Self {
        Self { width, color }
    }
}

impl Default for BorderSide {
    fn default() -> Self {
        Self::new(0.0, Color::BLACK)
    }
}

/// Independently configurable solid borders. The default has no visible edges.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Border {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}

impl Border {
    pub const fn all(width: f32, color: Color) -> Self {
        let side = BorderSide::new(width, color);
        Self {
            top: side,
            right: side,
            bottom: side,
            left: side,
        }
    }

    pub const fn top(mut self, width: f32, color: Color) -> Self {
        self.top = BorderSide::new(width, color);
        self
    }

    pub const fn right(mut self, width: f32, color: Color) -> Self {
        self.right = BorderSide::new(width, color);
        self
    }

    pub const fn bottom(mut self, width: f32, color: Color) -> Self {
        self.bottom = BorderSide::new(width, color);
        self
    }

    pub const fn left(mut self, width: f32, color: Color) -> Self {
        self.left = BorderSide::new(width, color);
        self
    }
}

/// Circular outer corner radii in logical pixels. Oversized radii are scaled
/// together by the host so adjacent corners fit the box, matching CSS.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadii {
    pub const fn all(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub const fn top_left(mut self, radius: f32) -> Self {
        self.top_left = radius;
        self
    }

    pub const fn top_right(mut self, radius: f32) -> Self {
        self.top_right = radius;
        self
    }

    pub const fn bottom_right(mut self, radius: f32) -> Self {
        self.bottom_right = radius;
        self
    }

    pub const fn bottom_left(mut self, radius: f32) -> Self {
        self.bottom_left = radius;
        self
    }
}

impl From<f32> for CornerRadii {
    fn from(radius: f32) -> Self {
        Self::all(radius)
    }
}

/// Alignment or distribution of content along a layout or text axis.
/// Available behaviors depend on the property receiving this value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    /// Place content at the start of the available space.
    #[default]
    Start,
    /// Center content within the available space.
    Center,
    /// Place content at the end of the available space.
    End,
    /// Stretch content across the available space where supported.
    Stretch,
    /// Distribute remaining space between children.
    SpaceBetween,
}

/// The horizontal, vertical, or combined axes enabled for scrolling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    /// Enable horizontal scrolling.
    Horizontal,
    /// Enable vertical scrolling.
    #[default]
    Vertical,
    /// Enable scrolling in both directions.
    Both,
}

/// Layout intent interpreted by the host adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Box,
    Row,
    Column,
    Overlay,
    Scroll(Axis),
}

/// Portable layout, paint, and typography properties.
/// Dimensions use logical pixels; spacing uses padding and gaps, without margins.
/// Optional values leave the corresponding choice to the adapter when absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub layout: Layout,
    pub padding: Insets,
    pub gap: f32,

    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,

    pub grow: f32,
    pub fill_width: bool,
    pub fill_height: bool,
    pub align: Alignment,
    pub justify: Alignment,

    pub background: Option<Color>,
    /// Painted over the background color, inside the same rounded border box.
    pub background_gradient: Option<LinearGradient>,
    pub shadow: Option<BoxShadow>,
    pub foreground: Option<Color>,
    pub border: Option<Border>,
    pub radius: CornerRadii,
    pub opacity: Option<f32>,
    pub clipped: bool,

    pub font_size: Option<f32>,
    pub font_weight: Option<u16>,
    pub font_family: Option<String>,
    pub line_height: Option<f32>,
    pub text_align: Option<Alignment>,
    pub truncate: bool,
    pub image_fit: crate::ImageFit,
}

impl Style {
    /// Reject invalid geometry and opacity before patches reach a host adapter.
    pub fn validate(&self) -> Result<(), ViewError> {
        if let Some(gradient) = &self.background_gradient {
            gradient.validate()?;
        }
        if let Some(shadow) = &self.shadow {
            shadow.validate()?;
        }
        let mut values = vec![
            self.padding.top,
            self.padding.right,
            self.padding.bottom,
            self.padding.left,
            self.gap,
            self.grow,
            self.radius.top_left,
            self.radius.top_right,
            self.radius.bottom_right,
            self.radius.bottom_left,
        ];
        if let Some(border) = self.border {
            values.extend([
                border.top.width,
                border.right.width,
                border.bottom.width,
                border.left.width,
            ]);
        }
        values.extend(
            [
                self.width,
                self.height,
                self.min_width,
                self.min_height,
                self.max_width,
                self.max_height,
                self.font_size,
                self.line_height,
            ]
            .into_iter()
            .flatten(),
        );
        if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(ViewError::new(
                "dimensions and insets must be finite and nonnegative",
            ));
        }

        for (min, max) in [
            (self.min_width, self.max_width),
            (self.min_height, self.max_height),
        ] {
            if matches!((min,max),(Some(a),Some(b)) if a>b) {
                return Err(ViewError::new("minimum dimension exceeds maximum"));
            }
        }

        if self
            .opacity
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err(ViewError::new("opacity must be between zero and one"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_border_edge_and_corner_rejects_invalid_geometry() {
        for invalid in [-1.0, f32::NAN, f32::INFINITY] {
            for side in 0..4 {
                let mut border = Border::default();
                [
                    &mut border.top,
                    &mut border.right,
                    &mut border.bottom,
                    &mut border.left,
                ][side]
                    .width = invalid;
                assert!(
                    Style {
                        border: Some(border),
                        ..Style::default()
                    }
                    .validate()
                    .is_err()
                );
                let mut radius = CornerRadii::default();
                [
                    &mut radius.top_left,
                    &mut radius.top_right,
                    &mut radius.bottom_right,
                    &mut radius.bottom_left,
                ][side]
                    .clone_from(&invalid);
                assert!(
                    Style {
                        radius,
                        ..Style::default()
                    }
                    .validate()
                    .is_err()
                );
            }
        }
        assert!(
            Style {
                border: Some(Border::default().left(4.0, Color::BLACK)),
                radius: CornerRadii::all(20.0).top_left(0.0),
                ..Style::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn rejects_invalid_stops_but_allows_hard_edges() {
        let mut gradient = LinearGradient::new(180.0, Color::BLACK, Color::WHITE);
        gradient.stops[1].offset = 0.0;
        assert!(gradient.validate().is_ok());
        for offset in [f32::NAN, -0.1, 1.1] {
            gradient.stops[1].offset = offset;
            assert!(gradient.validate().is_err());
        }
        gradient.stops = vec![
            GradientStop::new(0.8, Color::BLACK),
            GradientStop::new(0.2, Color::WHITE),
        ];
        assert!(gradient.validate().is_err());
        gradient.stops.clear();
        assert!(gradient.validate().is_err());
        assert!(
            LinearGradient::new(f32::INFINITY, Color::BLACK, Color::WHITE)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn css_angles_project_onto_non_square_boxes() {
        for (angle, expected) in [
            (0.0, ((100.0, 100.0), (100.0, 0.0))),
            (90.0, ((0.0, 50.0), (200.0, 50.0))),
            (180.0, ((100.0, 0.0), (100.0, 100.0))),
            (135.0, ((25.0, -25.0), (175.0, 125.0))),
            (495.0, ((25.0, -25.0), (175.0, 125.0))),
        ] {
            let (a, b) =
                LinearGradient::new(angle, Color::BLACK, Color::WHITE).endpoints(200.0, 100.0);
            assert!((a.0 - expected.0.0).abs() < 1e-6 && (a.1 - expected.0.1).abs() < 1e-6);
            assert!((b.0 - expected.1.0).abs() < 1e-6 && (b.1 - expected.1.1).abs() < 1e-6);
        }
    }

    #[test]
    fn signed_offsets_and_sharp_shadows_are_valid_but_invalid_geometry_is_rejected() {
        assert!(
            BoxShadow::new(-4.0, -2.0, 0.0, Color::BLACK)
                .validate()
                .is_ok()
        );
        for (x, y, blur) in [
            (f32::NAN, 0.0, 2.0),
            (0.0, f32::INFINITY, 2.0),
            (0.0, 0.0, -1.0),
            (0.0, 0.0, f32::NAN),
        ] {
            assert!(BoxShadow::new(x, y, blur, Color::BLACK).validate().is_err());
        }
    }
}

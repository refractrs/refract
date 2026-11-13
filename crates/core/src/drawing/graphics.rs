//! Portable drawing commands. Scoped state does not allocate compositing layers.
use super::{Path, Point, Rect};
use crate::{Affine2d, Color, GradientStop, ViewError};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
    },
    Radial {
        center: Point,
        radius: f64,
        stops: Vec<GradientStop>,
    },
}
impl Paint {
    pub fn linear(start: Point, end: Point, stops: impl Into<Vec<GradientStop>>) -> Self {
        Self::Linear {
            start,
            end,
            stops: stops.into(),
        }
    }
    pub fn radial(center: Point, radius: f64, stops: impl Into<Vec<GradientStop>>) -> Self {
        Self::Radial {
            center,
            radius,
            stops: stops.into(),
        }
    }
    fn valid(&self) -> bool {
        let stops = match self {
            Self::Solid(_) => return true,
            Self::Linear { start, end, stops } => {
                if ![start.x, start.y, end.x, end.y]
                    .iter()
                    .all(|v| v.is_finite())
                    || start == end
                {
                    return false;
                }
                stops
            }
            Self::Radial {
                center,
                radius,
                stops,
            } => {
                if ![center.x, center.y, *radius].iter().all(|v| v.is_finite()) || *radius <= 0.0 {
                    return false;
                }
                stops
            }
        };
        stops.len() >= 2
            && stops
                .iter()
                .all(|s| s.offset.is_finite() && (0.0..=1.0).contains(&s.offset))
            && stops.windows(2).all(|p| p[0].offset <= p[1].offset)
    }
}
impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}
impl From<&Paint> for Paint {
    fn from(paint: &Paint) -> Self {
        paint.clone()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f64,
    pub dash: Vec<f64>,
    pub dash_offset: f64,
}
impl StrokeStyle {
    pub fn new(width: f64) -> Self {
        Self {
            width,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 4.0,
            dash: vec![],
            dash_offset: 0.0,
        }
    }
    pub fn line_cap(mut self, cap: LineCap) -> Self {
        self.line_cap = cap;
        self
    }
    pub fn line_join(mut self, join: LineJoin) -> Self {
        self.line_join = join;
        self
    }
    pub fn dash(mut self, dash: impl Into<Vec<f64>>) -> Self {
        self.dash = dash.into();
        self
    }
    pub fn dash_offset(mut self, offset: f64) -> Self {
        self.dash_offset = offset;
        self
    }
    fn valid(&self) -> bool {
        self.width.is_finite()
            && self.width >= 0.0
            && self.miter_limit.is_finite()
            && self.miter_limit >= 1.0
            && self.dash_offset.is_finite()
            && self.dash.iter().all(|v| v.is_finite() && *v >= 0.0)
            && (self.dash.is_empty() || self.dash.iter().any(|v| *v > 0.0))
    }
}
impl From<&StrokeStyle> for StrokeStyle {
    fn from(style: &StrokeStyle) -> Self {
        style.clone()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Plus,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub path: Path,
    pub paint: Paint,
}
impl Mask {
    /// Alpha mask: paint RGB has no effect on coverage.
    pub fn new(path: Path, paint: impl Into<Paint>) -> Self {
        Self {
            path,
            paint: paint.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub opacity: f32,
    pub blur: f64,
    pub mask: Option<Mask>,
    pub blend_mode: BlendMode,
}
impl Default for Layer {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            blur: 0.0,
            mask: None,
            blend_mode: BlendMode::Normal,
        }
    }
}
impl Layer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
    /// Gaussian standard deviation in local logical pixels.
    pub fn blur(mut self, sigma: f64) -> Self {
        self.blur = sigma;
        self
    }
    pub fn mask(mut self, mask: Mask) -> Self {
        self.mask = Some(mask);
        self
    }
    pub fn blend_mode(mut self, mode: BlendMode) -> Self {
        self.blend_mode = mode;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawingCommand {
    Fill {
        path: Path,
        paint: Paint,
        rule: FillRule,
    },
    Stroke {
        path: Path,
        paint: Paint,
        style: StrokeStyle,
    },
    Group {
        transform: Affine2d,
        clip: Option<(Path, FillRule)>,
        layer: Option<Layer>,
        commands: Rc<[DrawingCommand]>,
    },
}
impl DrawingCommand {
    pub fn bounds(&self) -> Option<Rect> {
        match self {
            Self::Fill { path, .. } => path.bounds(),
            Self::Stroke { path, style, .. } => path
                .bounds()
                .map(|r| r.inflate(style.width * style.miter_limit / 2.0)),
            Self::Group {
                transform,
                layer,
                commands,
                ..
            } => drawing_bounds(commands).map(|r| {
                r.inflate(layer.as_ref().map_or(0.0, |l| l.blur * 3.0))
                    .transformed(*transform)
            }),
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Fill { path, paint, .. } => path.valid() && paint.valid(),
            Self::Stroke { path, paint, style } => path.valid() && paint.valid() && style.valid(),
            Self::Group {
                transform,
                clip,
                layer,
                commands,
            } => {
                transform.0.iter().all(|v| v.is_finite())
                    && clip.as_ref().is_none_or(|(p, _)| p.valid())
                    && layer.as_ref().is_none_or(|l| {
                        l.opacity.is_finite()
                            && (0.0..=1.0).contains(&l.opacity)
                            && l.blur.is_finite()
                            && l.blur >= 0.0
                            && l.mask
                                .as_ref()
                                .is_none_or(|m| m.path.valid() && m.paint.valid())
                    })
                    && commands.iter().all(Self::valid)
            }
        }
    }
}
pub fn drawing_bounds(commands: &[DrawingCommand]) -> Option<Rect> {
    commands
        .iter()
        .filter_map(DrawingCommand::bounds)
        .reduce(Rect::union)
}

#[derive(Default)]
pub struct GraphicsContext {
    commands: Vec<DrawingCommand>,
}
impl GraphicsContext {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn fill(&mut self, path: &Path, paint: impl Into<Paint>) {
        self.fill_rule(path, paint, FillRule::NonZero);
    }
    pub fn fill_rule(&mut self, path: &Path, paint: impl Into<Paint>, rule: FillRule) {
        self.commands.push(DrawingCommand::Fill {
            path: path.clone(),
            paint: paint.into(),
            rule,
        });
    }
    pub fn stroke(&mut self, path: &Path, paint: impl Into<Paint>, style: impl Into<StrokeStyle>) {
        self.commands.push(DrawingCommand::Stroke {
            path: path.clone(),
            paint: paint.into(),
            style: style.into(),
        });
    }
    pub fn scoped(&mut self, draw: impl FnOnce(&mut Self)) {
        self.group(Affine2d::IDENTITY, None, None, draw);
    }
    pub fn transformed(&mut self, transform: Affine2d, draw: impl FnOnce(&mut Self)) {
        self.group(transform, None, None, draw);
    }
    pub fn clipped(&mut self, path: &Path, rule: FillRule, draw: impl FnOnce(&mut Self)) {
        self.group(Affine2d::IDENTITY, Some((path.clone(), rule)), None, draw);
    }
    /// Isolate children, blur them, apply the alpha mask, then composite with
    /// group opacity and blend mode. Effects never change layout or hit testing.
    pub fn layer(&mut self, layer: Layer, draw: impl FnOnce(&mut Self)) {
        self.group(Affine2d::IDENTITY, None, Some(layer), draw);
    }
    fn group(
        &mut self,
        transform: Affine2d,
        clip: Option<(Path, FillRule)>,
        layer: Option<Layer>,
        draw: impl FnOnce(&mut Self),
    ) {
        let mut child = Self::new();
        draw(&mut child);
        self.commands.push(DrawingCommand::Group {
            transform,
            clip,
            layer,
            commands: child.commands.into(),
        });
    }
    /// Reject invalid user geometry before it reaches a rasterizer or DOM.
    pub fn finish(self) -> Result<Rc<[DrawingCommand]>, ViewError> {
        if !self.commands.iter().all(DrawingCommand::valid) {
            return Err(ViewError::new(
                "drawing contains invalid geometry, paint, or effects",
            ));
        }
        Ok(self.commands.into())
    }
}

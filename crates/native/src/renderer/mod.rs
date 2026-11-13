//! Renderer-independent drawing contract. Layout and shaping happen before this boundary.
//!
//! A renderer owns its surface/device/resources. It receives logical coordinates and
//! positioned glyphs, never Refract views or graph state. Replacing Vello therefore
//! does not require changing reconciliation, layout, input, or application code.

mod damage;
pub use damage::{Damage, DamageTracker, PixelRect};

#[cfg(feature = "vello")]
mod vello;
#[cfg(feature = "debug-damage")]
mod vello_debug;
#[cfg(feature = "vello")]
mod vello_effects;
#[cfg(feature = "vello")]
mod vello_geometry;
#[cfg(feature = "vello")]
mod vello_painter;
#[cfg(feature = "vello")]
mod vello_scene;
#[cfg(feature = "vello")]
mod vello_canvas;
#[cfg(feature = "vello")]
mod vello_shadow;

#[cfg(feature = "vello")]
pub use self::vello::VelloRenderer;
pub use parley::FontData;
#[cfg(feature = "vello")]
pub use vello_painter::{RenderMode, RenderStats};

use crate::Result;
use refract_core::{Border, Color, CornerRadii, Insets};
use std::{rc::Rc, sync::Arc};

/// A position or delta in logical pixels, with a top-left origin and y increasing downward.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// An axis-aligned box in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Includes the top/left edges and excludes the bottom/right edges.
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x < self.x + self.width
            && point.y < self.y + self.height
    }

    /// Return the shared region, clamped to zero size when disjoint.
    pub fn intersect(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self {
            x,
            y,
            width: ((self.x + self.width).min(other.x + other.width) - x).max(0.0),
            height: ((self.y + self.height).min(other.y + other.height) - y).max(0.0),
        }
    }
}

/// A shaped glyph's position relative to its text block, including its baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// Immutable glyph data retained across frames. Font coordinates are OpenType F2Dot14.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    pub font: FontData,
    pub font_size: f32,
    /// Suggested synthetic italic slant in degrees, applied in glyph-local coordinates.
    pub skew: Option<f32>,
    pub normalized_coords: Vec<i16>,
    pub glyphs: Vec<Glyph>,
}

/// Stable resource identity within one native host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageId(pub(crate) u64);

/// Immutable, tightly packed sRGB RGBA8 pixels with straight (unpremultiplied)
/// alpha. A renderer may cache its texture by `id`; unchanged frames share pixels.
#[derive(Clone)]
pub struct RasterImage {
    pub(crate) id: ImageId,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Arc<[u8]>,
}

impl std::fmt::Debug for RasterImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterImage")
            .field("id", &self.id)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl RasterImage {
    pub fn id(&self) -> ImageId {
        self.id
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn rgba(&self) -> &Arc<[u8]> {
        &self.rgba
    }
}

impl PartialEq for RasterImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// Commands are in painter's order. Every pushed layer has one matching pop.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    /// A canvas is clipped to its content box, including all filtered ink.
    Canvas {
        rect: Rect,
        commands: Rc<[refract_core::DrawingCommand]>,
    },
    /// Presentation transform around a subtree; pair with PopLayer.
    PushTransform(refract_core::Affine2d),
    /// Group opacity bounded only by the current viewport and ancestor clips.
    PushOpacity(f32),
    /// Isolated subtree: capture the backdrop before painting, optionally blur
    /// the completed subtree, then apply group opacity. Amounts are Gaussian
    /// standard deviations in logical pixels. Pair with `PopLayer`.
    PushEffects {
        rect: Rect,
        radius: CornerRadii,
        blur: f32,
        backdrop_blur: f32,
        opacity: f32,
    },
    /// Outer shadow behind the background; exclude the original rounded box's
    /// interior even when its background is transparent. Blur is CSS radius;
    /// shadow support ends at 1.5 times that radius beyond the offset box.
    BoxShadow {
        rect: Rect,
        radius: CornerRadii,
        shadow: refract_core::BoxShadow,
    },
    /// Draw the complete image into this destination. Cropping uses the surrounding clip.
    Image {
        rect: Rect,
        image: Arc<RasterImage>,
    },
    Fill {
        rect: Rect,
        radius: CornerRadii,
        color: Color,
    },
    /// A premultiplied-sRGB gradient clipped to the rounded border box.
    LinearGradient {
        rect: Rect,
        radius: CornerRadii,
        gradient: refract_core::LinearGradient,
    },
    /// A premultiplied-sRGB circular gradient clipped to the rounded border box.
    RadialGradient {
        rect: Rect,
        radius: CornerRadii,
        gradient: refract_core::RadialGradient,
    },
    Border {
        rect: Rect,
        radius: CornerRadii,
        border: Border,
    },
    /// Inset stroke, contained in the border box (like `Border`).
    Stroke {
        rect: Rect,
        radius: CornerRadii,
        width: f32,
        color: Color,
    },
    Glyphs {
        origin: Point,
        color: Color,
        run: Rc<GlyphRun>,
    },
    /// Clip plus group opacity; opacity applies to the composed subtree.
    PushLayer {
        clip: Rect,
        radius: CornerRadii,
        opacity: f32,
    },
    /// Clip to the rounded content box after subtracting padding and borders.
    PushInsetLayer {
        rect: Rect,
        radius: CornerRadii,
        insets: Insets,
    },
    PopLayer,
}

/// One complete frame description. Backends can use [`DamageTracker`] to retain
/// pixels and repaint only changed regions, or render the complete frame.
#[derive(Clone, Debug)]
pub struct Frame {
    pub commands: Vec<DrawCommand>,
    pub background: Color,
    /// Multiply logical coordinates by this factor to obtain physical pixels.
    pub scale_factor: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// Submitted and handed to the surface; not an acknowledgment that the display scanned it out.
    Presented,
    /// A temporary surface condition; the host schedules a bounded delayed retry.
    Retry,
}

/// Implement this trait to supply a different GPU renderer. Constructors receive
/// an `Arc<Window>` through `run_with_renderer`; no Vello types cross this interface.
pub trait Renderer {
    /// Physical pixel dimensions, always nonzero.
    fn resize(&mut self, width: u32, height: u32) -> Result<()>;
    fn render(&mut self, frame: &Frame) -> Result<Presentation>;

    /// Optional deadline for presentation-only work, such as expiring debug overlays.
    #[cfg(feature = "debug-damage")]
    fn redraw_deadline(&self) -> Option<std::time::Instant> {
        None
    }

    /// Refresh retained pixels without rebuilding a frame. Called when the deadline
    /// is due and the host has no scene changes. `None` falls back to normal rendering.
    #[cfg(feature = "debug-damage")]
    fn refresh(&mut self) -> Result<Option<Presentation>> {
        Ok(None)
    }
}

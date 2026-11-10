//! Renderer-independent drawing contract. Layout and shaping happen before this boundary.
//!
//! A renderer owns its surface/device/resources. It receives logical coordinates and
//! positioned glyphs, never Refract views or graph state. Replacing Vello therefore
//! does not require changing reconciliation, layout, input, or application code.

#[cfg(feature = "vello")]
mod vello;
#[cfg(feature = "vello")]
mod vello_shadow;
#[cfg(feature = "vello")]
mod vello_geometry;

#[cfg(feature = "vello")]
pub use self::vello::VelloRenderer;
pub use parley::FontData;

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
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// Immutable glyph data retained across frames. Font coordinates are OpenType F2Dot14.
#[derive(Clone, Debug)]
pub struct GlyphRun {
    pub font: FontData,
    pub font_size: f32,
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

/// Commands are in painter's order. Every pushed layer has one matching pop.
#[derive(Clone, Debug)]
pub enum DrawCommand {
    /// Outer shadow behind the background; exclude the original rounded box's
    /// interior even when its background is transparent. Blur is CSS radius.
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

/// One complete frame description. A GPU renderer may repaint the full surface;
/// retained text resources and incremental model evaluation are independent of that.
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
}

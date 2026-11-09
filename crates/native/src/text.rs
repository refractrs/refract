//! Parley shaping with per-element caches. Width changes reflow without reshaping.

use crate::renderer::{Glyph, GlyphRun};
use parley::{AlignmentOptions, FontContext, LayoutContext, PositionedLayoutItem, StyleProperty};
use refract_core::{Alignment, Color, Style};
use std::rc::Rc;

/// Resolved inherited text properties. Paint color is deliberately outside the
/// shaping key: changing a color should not rebuild glyph layout.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Typography {
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub line_height: Option<f32>,
    pub align: Alignment,
    pub color: Color,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            family: "system-ui, sans-serif".into(),
            size: 14.0,
            weight: 400,
            line_height: None,
            align: Alignment::Start,
            color: Color::BLACK,
        }
    }
}

impl Typography {
    pub fn inherit(&self, style: &Style) -> Self {
        Self {
            family: style
                .font_family
                .clone()
                .unwrap_or_else(|| self.family.clone()),
            size: style.font_size.unwrap_or(self.size),
            weight: style.font_weight.unwrap_or(self.weight),
            line_height: style.line_height.or(self.line_height),
            align: style.text_align.unwrap_or(self.align),
            color: style.foreground.unwrap_or(self.color),
        }
    }

    fn same_shape(&self, other: &Self) -> bool {
        self.family == other.family
            && self.size == other.size
            && self.weight == other.weight
            && self.line_height == other.line_height
    }
}

#[derive(Default)]
pub(crate) struct TextSystem {
    pub(crate) fonts: FontContext,
    pub(crate) context: LayoutContext<()>,
    pub shapes: u64,
}

pub(crate) struct TextBlock {
    text: String,
    typography: Typography,
    layout: parley::Layout<()>,
    content_widths: parley::ContentWidths,
    width: Option<f32>,
    laid_out: bool,
    runs: Option<Vec<Rc<GlyphRun>>>,
}

impl TextSystem {
    pub fn shape(&mut self, text: &str, typography: &Typography) -> TextBlock {
        let mut builder = self
            .context
            .ranged_builder(&mut self.fonts, text, 1.0, false);
        builder.push_default(StyleProperty::FontSize(typography.size));
        builder.push_default(StyleProperty::FontWeight(parley::FontWeight::new(
            typography.weight as f32,
        )));
        builder.push_default(StyleProperty::FontFamily(typography.family.as_str().into()));
        if let Some(height) = typography.line_height {
            builder.push_default(StyleProperty::LineHeight(parley::LineHeight::Absolute(
                height,
            )));
        }
        let layout = builder.build(text);
        // Intrinsic widths depend on the shaped text, not its current wrapping.
        // Cache them once so resize measurement stays independent of paint order.
        let content_widths = layout.calculate_content_widths();
        self.shapes += 1;
        TextBlock {
            text: text.into(),
            typography: typography.clone(),
            layout,
            content_widths,
            width: None,
            laid_out: false,
            runs: None,
        }
    }
}

impl TextBlock {
    pub fn matches(&self, text: &str, typography: &Typography) -> bool {
        self.text == text && self.typography.same_shape(typography)
    }

    pub fn content_widths(&self) -> (f32, f32) {
        (self.content_widths.min, self.content_widths.max)
    }

    pub fn measure(&mut self, width: Option<f32>, align: Alignment) -> (f32, f32) {
        if !self.laid_out || self.width != width || self.typography.align != align {
            self.layout.break_all_lines(width);
            self.layout.align(
                match align {
                    Alignment::Center => parley::Alignment::Center,
                    Alignment::End => parley::Alignment::End,
                    _ => parley::Alignment::Start,
                },
                AlignmentOptions::default(),
            );
            self.width = width;
            self.typography.align = align;
            self.laid_out = true;
            self.runs = None;
        }
        (self.layout.width(), self.layout.height())
    }

    pub fn glyph_runs(&mut self) -> &[Rc<GlyphRun>] {
        self.runs.get_or_insert_with(|| glyph_runs(&self.layout))
    }
}

/// Both static text and editable text share renderer-neutral glyph extraction.
pub(crate) fn glyph_runs(layout: &parley::Layout<()>) -> Vec<Rc<GlyphRun>> {
    layout
        .lines()
        .flat_map(|line| line.items())
        .filter_map(|item| {
            let PositionedLayoutItem::GlyphRun(glyphs) = item else {
                return None;
            };
            let run = glyphs.run();
            Some(Rc::new(GlyphRun {
                font: run.font().clone(),
                font_size: run.font_size(),
                normalized_coords: run
                    .normalized_coords()
                    .iter()
                    .map(|c| c.to_bits())
                    .collect(),
                glyphs: glyphs
                    .positioned_glyphs()
                    .map(|g| Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    })
                    .collect(),
            }))
        })
        .collect()
}

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
    pub italic: bool,
    pub letter_spacing: f32,
    pub word_spacing: f32,
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
            italic: false,
            letter_spacing: 0.0,
            word_spacing: 0.0,
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
            italic: style.italic.unwrap_or(self.italic),
            letter_spacing: style.letter_spacing.unwrap_or(self.letter_spacing),
            word_spacing: style.word_spacing.unwrap_or(self.word_spacing),
            line_height: style.line_height.or(self.line_height),
            align: style.text_align.unwrap_or(self.align),
            color: style.foreground.unwrap_or(self.color),
        }
    }

    fn same_shape(&self, other: &Self) -> bool {
        self.family == other.family
            && self.size == other.size
            && self.weight == other.weight
            && self.italic == other.italic
            && self.letter_spacing == other.letter_spacing
            && self.word_spacing == other.word_spacing
            && self.line_height == other.line_height
    }

    pub fn font_style(&self) -> parley::FontStyle {
        if self.italic {
            parley::FontStyle::Italic
        } else {
            parley::FontStyle::Normal
        }
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
        builder.push_default(StyleProperty::FontStyle(typography.font_style()));
        builder.push_default(StyleProperty::LetterSpacing(typography.letter_spacing));
        builder.push_default(StyleProperty::WordSpacing(typography.word_spacing));
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
                skew: run.synthesis().skew(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typography_inherits_and_explicit_values_override_it() {
        let inherited = Typography {
            italic: true,
            letter_spacing: 1.5,
            word_spacing: -0.5,
            ..Default::default()
        };
        assert_eq!(inherited.inherit(&Style::default()), inherited);
        let reset = inherited.inherit(&Style {
            italic: Some(false),
            letter_spacing: Some(0.0),
            word_spacing: Some(0.0),
            ..Default::default()
        });
        assert_eq!(reset, Typography::default());
    }

    #[test]
    fn italic_synthesis_survives_glyph_extraction() {
        use parley::fontique::{Collection, CollectionOptions, FontInfoOverride};

        // Reuse an installed static monospace font, but register only upright
        // faces in an isolated collection so italic must use synthesis.
        let mut system = TextSystem::default();
        let typography = Typography {
            family: "'Courier New', 'DejaVu Sans Mono', monospace".into(),
            ..Default::default()
        };
        let mut regular = system.shape("Hello", &typography);
        regular.measure(None, Alignment::Start);
        let font = regular.glyph_runs()[0].font.clone();
        let mut isolated = TextSystem::default();
        isolated.fonts.collection = Collection::new(CollectionOptions {
            system_fonts: false,
            ..Default::default()
        });
        let registered = isolated.fonts.collection.register_fonts(
            font.data,
            Some(FontInfoOverride {
                family_name: Some("Refract Test Upright"),
                style: Some(parley::FontStyle::Normal),
                ..Default::default()
            }),
        );
        assert!(!registered.is_empty());
        for italic in [false, true] {
            let mut block = isolated.shape(
                "Hello",
                &Typography {
                    family: "'Refract Test Upright'".into(),
                    italic,
                    ..Default::default()
                },
            );
            block.measure(None, Alignment::Start);
            let runs = block.glyph_runs();
            assert!(!runs.is_empty());
            assert!(runs.iter().all(|run| run.skew.is_some() == italic));
        }
    }
}

//! Parley shaping with per-element caches. Wrapping reflows without reshaping;
//! truncated labels cache ellipsized glyphs for their current width and alignment.

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
    pub truncate: bool,
    single_line: Option<String>,
    typography: Typography,
    layout: parley::Layout<()>,
    content_widths: parley::ContentWidths,
    width: Option<f32>,
    laid_out: bool,
    runs: Option<Vec<Rc<GlyphRun>>>,
}

impl TextSystem {
    pub fn shape(&mut self, text: &str, typography: &Typography) -> TextBlock {
        self.shape_label(text, typography, false)
    }

    pub fn shape_label(
        &mut self,
        text: &str,
        typography: &Typography,
        truncate: bool,
    ) -> TextBlock {
        // Match the web's white-space: nowrap. RangedBuilder expects whitespace
        // to be collapsed before shaping; nonbreaking spaces remain significant.
        let single_line = truncate.then(|| {
            text.split([' ', '\t', '\n', '\r', '\u{000c}'])
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        });
        let shaped_text = single_line.as_deref().unwrap_or(text);
        let mut builder = self
            .context
            .ranged_builder(&mut self.fonts, shaped_text, 1.0, false);
        if truncate {
            builder.push_default(StyleProperty::TextWrapMode(parley::TextWrapMode::NoWrap));
        }
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
        let layout = builder.build(shaped_text);
        // Intrinsic widths depend on the shaped text, not its current wrapping.
        // Cache them once so resize measurement stays independent of paint order.
        let content_widths = layout.calculate_content_widths();
        self.shapes += 1;
        TextBlock {
            text: text.into(),
            truncate,
            single_line,
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
        (
            if self.truncate {
                0.0
            } else {
                self.content_widths.min
            },
            self.content_widths.max,
        )
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

    /// Keep intrinsic measurement based on the full text. Only the painted
    /// string is shortened, and its glyphs are cached until width/alignment changes.
    pub fn prepare_ellipsis(&mut self, system: &mut TextSystem) {
        if !self.truncate || self.runs.is_some() {
            return;
        }
        let Some(width) = self.width else { return };
        if self.layout.width() <= width {
            return;
        }
        let mut shortened = self.ellipsized(system, width);
        shortened.measure(Some(width), self.typography.align);
        self.runs = Some(shortened.glyph_runs().to_vec());
    }

    fn ellipsized(&self, system: &mut TextSystem, width: f32) -> TextBlock {
        let text = self.single_line.as_deref().unwrap_or(&self.text);
        // Parley's cluster ranges are grapheme boundaries, including combining
        // marks and joined emoji. Sort logical offsets because runs may be bidi.
        let mut ends: Vec<_> = self
            .layout
            .lines()
            .flat_map(|line| line.runs())
            .flat_map(|run| run.clusters().map(|cluster| cluster.text_range().end))
            .collect();
        ends.sort_unstable();
        ends.dedup();
        let mut best = system.shape_label("…", &self.typography, true);
        best.measure(None, Alignment::Start);
        if best.layout.width() > width {
            // As on the web, the marker itself is clipped when it cannot fit.
            return best;
        }
        let (mut low, mut high) = (0, ends.len());
        while low < high {
            let middle = low + (high - low) / 2;
            let candidate = format!("{}…", &text[..ends[middle]]);
            let mut block = system.shape_label(&candidate, &self.typography, true);
            block.measure(None, Alignment::Start);
            if block.layout.width() <= width {
                best = block;
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        best
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
    fn truncation_collapses_whitespace_and_keeps_one_line() {
        let mut system = TextSystem::default();
        let mut block = system.shape_label("  one\t two\r\nthree  ", &Typography::default(), true);
        let (_, height) = block.measure(Some(20.0), Alignment::Start);
        assert_eq!(block.single_line.as_deref(), Some("one two three"));
        assert_eq!(block.layout.len(), 1);
        assert!(height > 0.0);
        assert_eq!(block.content_widths().0, 0.0);
        let intrinsic = block.content_widths();
        block.prepare_ellipsis(&mut system);
        assert_eq!(block.content_widths(), intrinsic);

        let mut normal = system.shape("one\ntwo", &Typography::default());
        normal.measure(Some(200.0), Alignment::Start);
        assert_eq!(normal.layout.len(), 2);
        let nbsp = system.shape_label("one\u{a0}two", &Typography::default(), true);
        assert_eq!(nbsp.single_line.as_deref(), Some("one\u{a0}two"));
    }

    #[test]
    fn ellipsis_preserves_graphemes_and_fits_the_available_width() {
        let mut system = TextSystem::default();
        for parts in [
            vec!["A", "e\u{301}", "👨‍👩‍👧‍👦", "中", "B"],
            vec!["ש", "ל", "ו", "ם", " ", "ע", "ו", "ל", "ם"],
        ] {
            let value = parts.concat();
            let mut block = system.shape_label(&value, &Typography::default(), true);
            block.measure(None, Alignment::Start);
            let mut marker = system.shape("…", &Typography::default());
            let marker_width = marker.measure(None, Alignment::Start).0;
            for width in [0.0, marker_width / 2.0, marker_width, 30.0, 50.0, 70.0] {
                let clipped = block.ellipsized(&mut system, width);
                let prefix = clipped.text.strip_suffix('…').unwrap();
                assert!(
                    (0..=parts.len()).any(|n| parts[..n].concat() == prefix),
                    "split grapheme: {prefix:?}"
                );
                if width >= marker_width {
                    assert!(
                        clipped.layout.width() <= width,
                        "{} > {width}",
                        clipped.layout.width()
                    );
                }
                assert_eq!(clipped.layout.len(), 1);
            }
        }
    }

    #[test]
    fn truncation_cache_tracks_width_and_alignment_without_losing_full_text() {
        let mut system = TextSystem::default();
        let mut block = system.shape_label(
            "A long label that must be shortened",
            &Typography::default(),
            true,
        );
        block.measure(Some(80.0), Alignment::Start);
        block.prepare_ellipsis(&mut system);
        let first = block.glyph_runs()[0].clone();
        let shapes = system.shapes;
        block.measure(Some(80.0), Alignment::Start);
        block.prepare_ellipsis(&mut system);
        assert_eq!(system.shapes, shapes);
        assert!(Rc::ptr_eq(&first, &block.glyph_runs()[0]));

        block.measure(Some(80.0), Alignment::End);
        block.prepare_ellipsis(&mut system);
        assert!(!Rc::ptr_eq(&first, &block.glyph_runs()[0]));
        let intrinsic = block.content_widths().1;
        block.measure(Some(intrinsic + 100.0), Alignment::Start);
        block.prepare_ellipsis(&mut system);
        let full: usize = block.glyph_runs().iter().map(|r| r.glyphs.len()).sum();
        assert!(full > first.glyphs.len());
        block.measure(Some(0.0), Alignment::Start);
        block.prepare_ellipsis(&mut system);
        assert!(block.glyph_runs().iter().all(|run| run
            .glyphs
            .iter()
            .all(|g| g.x.is_finite() && g.y.is_finite())));
    }

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

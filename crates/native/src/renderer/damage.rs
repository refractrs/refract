//! Conservative damage planning, independent of the rasterizer and GPU API.
//! Compare against the last *committed* frame, never an attempted presentation.

use super::{DrawCommand, Frame, GlyphRun, Rect};
use skrifa::{instance::Size, raw::TableProvider, FontRef, GlyphId, MetadataProvider};
use std::{
    collections::HashMap,
    rc::{Rc, Weak},
};

/// A nonempty, pixel-aligned region of the physical render target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Work needed to bring retained pixels up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    None,
    Full,
    Region(PixelRect),
}

type GlyphBounds = HashMap<usize, (Weak<GlyphRun>, Option<Rect>)>;

/// Optional incremental rendering support for any [`super::Renderer`].
///
/// Call `plan`, repaint the returned region from the frame background in painter's
/// order, then `commit` only after successful rasterization. Present the retained
/// target even for `None`: swapchain images need not preserve previous pixels.
/// Invalidate when the retained target is lost. Resize, DPI and background changes
/// are detected automatically. Structural/layer changes and filters deliberately
/// favor correctness over minimal damage. Bounds follow the drawing contract:
/// primitives stay inside their boxes, glyphs use unhinted font outlines, and
/// antialiasing extends coverage by at most one physical pixel. A backend with
/// wider reconstruction filters must expand damage and selection accordingly.
#[derive(Default)]
pub struct DamageTracker {
    previous: Option<(Frame, u32, u32)>,
    glyphs: GlyphBounds,
}

impl DamageTracker {
    pub fn invalidate(&mut self) {
        self.previous = None;
        self.glyphs.clear();
    }

    pub fn commit(&mut self, frame: &Frame, width: u32, height: u32) {
        self.previous = Some((frame.clone(), width, height));
        self.glyphs.retain(|_, (run, _)| run.strong_count() > 0);
    }

    pub fn plan(&mut self, frame: &Frame, width: u32, height: u32) -> Damage {
        let Some((old, w, h)) = &self.previous else {
            return Damage::Full;
        };
        if (*w, *h) != (width, height)
            || old.scale_factor != frame.scale_factor
            || old.background != frame.background
            || !frame.scale_factor.is_finite()
            || frame.scale_factor <= 0.0
        {
            return Damage::Full;
        }
        let changed = changed_commands(&old.commands, &frame.commands);
        if changed.is_empty() {
            return Damage::None;
        }
        let mut bounds = None;
        for command in changed {
            // Changing an ancestor changes how every descendant is composed.
            // Its own box does not necessarily bound overflowing content.
            if is_layer(command) {
                return Damage::Full;
            }
            let Some(rect) = command_bounds(command, &mut self.glyphs) else {
                return Damage::Full;
            };
            if [rect.x, rect.y, rect.width, rect.height]
                .iter()
                .any(|v| !v.is_finite())
            {
                return Damage::Full;
            }
            bounds = union(bounds, rect);
        }
        // Filters can sample outside damage, and backdrop filters depend on earlier
        // siblings. Until dependencies are modeled, even an unchanged filter makes
        // any changed frame a full repaint.
        if frame
            .commands
            .iter()
            .any(|c| matches!(c, DrawCommand::PushEffects { .. } | DrawCommand::PushTransform(_)))
        {
            return Damage::Full;
        }
        let Some(rect) = bounds else {
            return Damage::None;
        };
        match pixels(rect, frame.scale_factor, width, height) {
            None => Damage::None,
            Some(r) if r.width == width && r.height == height => Damage::Full,
            Some(r) => Damage::Region(r),
        }
    }

    /// Select intersecting primitives and all layer delimiters. This preserves
    /// painter's order, ancestor clipping and group opacity. Bounds are conservative;
    /// unknown ink (e.g. color fonts) is always included. Use only for filter-free
    /// regional repaints; `Full` must pass the original commands to the backend.
    pub fn commands_for_region(&mut self, frame: &Frame, region: PixelRect) -> Vec<DrawCommand> {
        frame
            .commands
            .iter()
            .filter(|command| {
                if is_layer(command) {
                    return true;
                }
                let Some(rect) = command_bounds(command, &mut self.glyphs) else {
                    return true;
                };
                let Some(r) = pixels(rect, frame.scale_factor, u32::MAX, u32::MAX) else {
                    return false;
                };
                u64::from(r.x) < u64::from(region.x) + u64::from(region.width)
                    && u64::from(region.x) < u64::from(r.x) + u64::from(r.width)
                    && u64::from(r.y) < u64::from(region.y) + u64::from(region.height)
                    && u64::from(region.y) < u64::from(r.y) + u64::from(r.height)
            })
            .cloned()
            .collect()
    }
}

// Trim shared ends so inserting/removing a caret, selection or hover overlay
// does not invalidate the rest of the display list. Multiple structural edits
// may repaint extra content, but never require identity matching or hashing.
fn changed_commands<'a>(old: &'a [DrawCommand], new: &'a [DrawCommand]) -> Vec<&'a DrawCommand> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let old = &old[prefix..];
    let new = &new[prefix..];
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old = &old[..old.len() - suffix];
    let new = &new[..new.len() - suffix];
    if old.len() == new.len() {
        old.iter()
            .zip(new)
            .filter(|(a, b)| a != b)
            .flat_map(|(a, b)| [a, b])
            .collect()
    } else {
        old.iter().chain(new).collect()
    }
}

fn is_layer(command: &DrawCommand) -> bool {
    matches!(
        command,
        DrawCommand::PushTransform(_) | DrawCommand::PushOpacity(_) | DrawCommand::PushLayer { .. }
            | DrawCommand::PushInsetLayer { .. }
            | DrawCommand::PushEffects { .. }
            | DrawCommand::PopLayer
    )
}

fn command_bounds(command: &DrawCommand, cache: &mut GlyphBounds) -> Option<Rect> {
    match command {
        DrawCommand::Canvas { rect, .. }
        | DrawCommand::Fill { rect, .. }
        | DrawCommand::LinearGradient { rect, .. }
        | DrawCommand::RadialGradient { rect, .. }
        | DrawCommand::Image { rect, .. }
        | DrawCommand::Border { rect, .. }
        | DrawCommand::Stroke { rect, .. } => Some(*rect),
        DrawCommand::BoxShadow { rect, shadow, .. } => {
            let pad = shadow.blur_radius.max(0.0) * 1.5;
            union(
                Some(*rect),
                Rect {
                    x: rect.x + shadow.offset_x - pad,
                    y: rect.y + shadow.offset_y - pad,
                    width: rect.width + 2.0 * pad,
                    height: rect.height + 2.0 * pad,
                },
            )
        }
        DrawCommand::Glyphs { origin, run, .. } => {
            let key = Rc::as_ptr(run) as usize;
            let entry = cache
                .entry(key)
                .or_insert_with(|| (Rc::downgrade(run), glyph_bounds(run)));
            // Weak references keep the allocation address reserved until eviction.
            entry.1.map(|r| Rect {
                x: r.x + origin.x,
                y: r.y + origin.y,
                ..r
            })
        }
        _ => None,
    }
}

fn glyph_bounds(run: &GlyphRun) -> Option<Rect> {
    let font = FontRef::from_index(run.font.data.as_ref(), run.font.index).ok()?;
    // Outlines alone do not bound bitmap/color glyph paint graphs.
    if font.colr().is_ok() || font.sbix().is_ok() || font.cbdt().is_ok() || font.ebdt().is_ok() {
        return None;
    }
    let coords: Vec<_> = run
        .normalized_coords
        .iter()
        .map(|c| skrifa::instance::NormalizedCoord::from_bits(*c))
        .collect();
    let metrics = font.glyph_metrics(Size::new(run.font_size), coords.as_slice());
    let skew = run.skew.unwrap_or(0.0).to_radians().tan();
    let mut bounds = None;
    for g in &run.glyphs {
        let b = match metrics.bounds(GlyphId::new(g.id)) {
            Some(bounds) => bounds,
            None => {
                // CFF/variable fonts report no bounds for valid empty outlines
                // (notably spaces). Distinguish those from unreadable glyphs.
                let outlines = font.outline_glyphs();
                let outline = outlines.get(GlyphId::new(g.id))?;
                let mut pen = skrifa::outline::pen::ControlBoundsPen::default();
                outline
                    .draw(
                        skrifa::outline::DrawSettings::unhinted(
                            Size::new(run.font_size),
                            coords.as_slice(),
                        ),
                        &mut pen,
                    )
                    .ok()?;
                match pen.bounding_box() {
                    Some(bounds) => bounds,
                    None => continue,
                }
            }
        };
        let xs = [
            b.x_min + skew * b.y_min,
            b.x_min + skew * b.y_max,
            b.x_max + skew * b.y_min,
            b.x_max + skew * b.y_max,
        ];
        let left = xs.into_iter().fold(f32::INFINITY, f32::min);
        let right = xs.into_iter().fold(f32::NEG_INFINITY, f32::max);
        bounds = union(
            bounds,
            Rect {
                x: g.x + left,
                y: g.y - b.y_max,
                width: right - left,
                height: b.y_max - b.y_min,
            },
        );
    }
    Some(bounds.unwrap_or_default())
}

fn union(bounds: Option<Rect>, rect: Rect) -> Option<Rect> {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return bounds;
    }
    Some(match bounds {
        None => rect,
        Some(a) => {
            let x = a.x.min(rect.x);
            let y = a.y.min(rect.y);
            Rect {
                x,
                y,
                width: (a.x + a.width).max(rect.x + rect.width) - x,
                height: (a.y + a.height).max(rect.y + rect.height) - y,
            }
        }
    })
}

fn pixels(rect: Rect, scale: f64, width: u32, height: u32) -> Option<PixelRect> {
    if [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .any(|v| !v.is_finite())
    {
        return Some(PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        });
    }
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    // One physical pixel of guard coverage for antialiasing and font metric rounding.
    let x = (f64::from(rect.x) * scale - 1.0)
        .floor()
        .clamp(0.0, width as f64) as u32;
    let y = (f64::from(rect.y) * scale - 1.0)
        .floor()
        .clamp(0.0, height as f64) as u32;
    let right = ((f64::from(rect.x) + f64::from(rect.width)) * scale + 1.0)
        .ceil()
        .clamp(0.0, width as f64) as u32;
    let bottom = ((f64::from(rect.y) + f64::from(rect.height)) * scale + 1.0)
        .ceil()
        .clamp(0.0, height as f64) as u32;
    (right > x && bottom > y).then_some(PixelRect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::Color;

    fn fill(x: f32) -> DrawCommand {
        DrawCommand::Fill {
            rect: Rect {
                x,
                y: 10.0,
                width: 10.0,
                height: 10.0,
            },
            radius: 0.0.into(),
            color: Color::BLACK,
        }
    }
    fn frame(commands: Vec<DrawCommand>) -> Frame {
        Frame {
            commands,
            background: Color::WHITE,
            scale_factor: 1.0,
        }
    }

    #[test]
    fn damage_includes_old_and_new_pixels_and_waits_for_commit() {
        let mut tracker = DamageTracker::default();
        let old = frame(vec![fill(10.0)]);
        let next = frame(vec![fill(30.0)]);
        assert_eq!(tracker.plan(&old, 100, 100), Damage::Full);
        tracker.commit(&old, 100, 100);
        assert_eq!(tracker.plan(&old, 100, 100), Damage::None);
        let expected = Damage::Region(PixelRect {
            x: 9,
            y: 9,
            width: 32,
            height: 12,
        });
        assert_eq!(tracker.plan(&next, 100, 100), expected);
        assert_eq!(
            tracker.plan(&next, 100, 100),
            expected,
            "retry must retain pending damage"
        );
        tracker.commit(&next, 100, 100);
        assert_eq!(tracker.plan(&next, 100, 100), Damage::None);
        tracker.invalidate();
        assert_eq!(tracker.plan(&next, 100, 100), Damage::Full);
    }

    #[test]
    fn global_changes_repaint_fully_and_removal_clears_old_ink() {
        let old = frame(vec![fill(10.0)]);
        let mut tracker = DamageTracker::default();
        tracker.commit(&old, 100, 100);
        assert_eq!(tracker.plan(&old, 200, 100), Damage::Full);
        let mut next = old.clone();
        next.scale_factor = 2.0;
        assert_eq!(tracker.plan(&next, 100, 100), Damage::Full);
        next = old.clone();
        next.background = Color::BLACK;
        assert_eq!(tracker.plan(&next, 100, 100), Damage::Full);
        next = frame(vec![]);
        assert_eq!(
            tracker.plan(&next, 100, 100),
            Damage::Region(PixelRect {
                x: 9,
                y: 9,
                width: 12,
                height: 12
            })
        );
    }

    #[test]
    fn layers_filters_and_fractional_scale_are_conservative() {
        let layer = DrawCommand::PushLayer {
            clip: Rect {
                width: 80.0,
                height: 80.0,
                ..Default::default()
            },
            radius: 9.0.into(),
            opacity: 0.5,
        };
        let old = frame(vec![layer, fill(10.0), DrawCommand::PopLayer, fill(90.0)]);
        let mut next = old.clone();
        next.commands[1] = fill(12.0);
        let mut tracker = DamageTracker::default();
        tracker.commit(&old, 100, 100);
        let Damage::Region(region) = tracker.plan(&next, 100, 100) else {
            panic!("local change")
        };
        let selected = tracker.commands_for_region(&next, region);
        assert_eq!(
            selected,
            next.commands[..3],
            "retain layers and omit disjoint sibling"
        );
        if let DrawCommand::PushLayer { opacity, .. } = &mut next.commands[0] {
            *opacity = 0.8;
        }
        assert_eq!(tracker.plan(&next, 100, 100), Damage::Full);
        let mut filtered = old.clone();
        filtered.commands[0] = DrawCommand::PushEffects {
            rect: Rect::default(),
            radius: 0.0.into(),
            blur: 3.0,
            backdrop_blur: 0.0,
            opacity: 1.0,
        };
        tracker.commit(&filtered, 100, 100);
        assert_eq!(tracker.plan(&filtered, 100, 100), Damage::None);
        filtered.commands[3] = fill(92.0);
        assert_eq!(tracker.plan(&filtered, 100, 100), Damage::Full);
        assert_eq!(
            pixels(
                Rect {
                    x: 1.5,
                    y: 2.5,
                    width: 3.0,
                    height: 4.0
                },
                1.5,
                100,
                100
            ),
            Some(PixelRect {
                x: 1,
                y: 2,
                width: 7,
                height: 9
            })
        );
    }

    #[test]
    fn offscreen_changes_do_not_rasterize_and_nonfinite_bounds_cover_the_target() {
        let mut tracker = DamageTracker::default();
        tracker.commit(&frame(vec![fill(-50.0)]), 100, 100);
        assert_eq!(
            tracker.plan(&frame(vec![fill(-40.0)]), 100, 100),
            Damage::None
        );
        assert_eq!(
            pixels(
                Rect {
                    x: f32::NAN,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0
                },
                1.0,
                100,
                100
            ),
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 100,
                height: 100
            })
        );
    }
}

#[cfg(test)]
mod canvas_tests {
    use super::*;
    #[test]
    fn canvas_effect_damage_stays_inside_the_canvas_clip() {
        let frame=|x| {
            let mut g=refract_core::GraphicsContext::new();
            g.layer(refract_core::Layer::new().blur(12.0),|g|g.fill(&refract_core::Path::rectangle(refract_core::Rect::new(x,0.0,10.0,10.0)),refract_core::Color::BLUE));
            Frame {commands:vec![DrawCommand::Canvas {rect:Rect {x:20.0,y:30.0,width:100.0,height:80.0},commands:g.finish().unwrap()}],background:refract_core::Color::WHITE,scale_factor:1.0}
        };
        let mut damage=DamageTracker::default();
        damage.commit(&frame(0.0),400,400);
        assert_eq!(damage.plan(&frame(5.0),400,400),Damage::Region(PixelRect {x:19,y:29,width:102,height:82}));
    }
}

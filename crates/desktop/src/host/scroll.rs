//! Custom scroll surfaces consume OS deltas unchanged. In particular, precise
//! macOS wheel events already include momentum; adding velocity here doubles it.
//! Offsets, pointer capture and scrollbar geometry never enter the app graph.

mod presentation;

use super::{Host, RetainedElement};
use crate::renderer::{DrawCommand, Frame, Point, Rect};
use presentation::Presentation;
use refract_core::{Axis, Color, ElementId, Layout};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use winit::event::TouchPhase;

#[derive(Clone, Copy)]
struct Scrollbar {
    target: ElementId,
    vertical: bool,
    track: Rect,
    thumb: Rect,
    clip: Rect,
    max: f32,
}

#[derive(Clone, Copy)]
struct Drag {
    bar: Scrollbar,
    grab: f32,
    pointer: Point,
}

#[derive(Default)]
pub(super) struct ScrollState {
    bars: Vec<Scrollbar>,
    target: Option<ElementId>,
    last_event: Option<Instant>,
    drag: Option<Drag>,
    hover: Option<(ElementId, bool)>,
    presentation: HashMap<(ElementId, bool), Presentation>,
    frame_time: Option<Instant>,
    resized: bool,
    deadline: Option<Instant>,
}

impl ScrollState {
    pub fn begin_frame(&mut self, now: Instant, resized: bool) {
        self.bars.clear();
        self.frame_time = Some(now);
        self.resized = resized;
        for presentation in self.presentation.values_mut() {
            presentation.seen = false;
        }
    }

    pub fn end_frame(&mut self) {
        self.presentation
            .retain(|_, presentation| presentation.seen);
        if self
            .hover
            .is_some_and(|key| !self.presentation.contains_key(&key))
        {
            self.hover = None;
        }
        if self.drag.is_some_and(|drag| {
            !self
                .presentation
                .contains_key(&(drag.bar.target, drag.bar.vertical))
        }) {
            self.drag = None;
        }
        let now = self.frame_time.unwrap();
        self.deadline = self
            .presentation
            .values()
            .filter_map(|p| p.deadline(now))
            .min();
    }

    fn reveal(&mut self, id: ElementId, now: Instant) -> bool {
        let mut changed = false;
        for ((target, _), presentation) in &mut self.presentation {
            if *target == id {
                presentation.reveal(now);
                changed = true;
            }
        }
        changed
    }
}

impl RetainedElement {
    fn scroll_geometry(&self) -> Option<(Axis, Rect, Point, Point)> {
        if let Some(editor) = &self.editor {
            let (bounds, offset, max) = editor.scroll_geometry();
            return Some((Axis::Both, bounds, offset, max));
        }
        let Layout::Scroll(axis) = self.element.style.layout else {
            return None;
        };
        Some((axis, self.rect, self.scroll, self.scroll_max))
    }

    fn set_scroll(&mut self, offset: Point) {
        if let Some(editor) = &mut self.editor {
            editor.set_scroll(offset);
        } else {
            self.scroll = offset;
        }
    }
}

impl Host {
    /// Start at the deepest visible scroll surface. If it is at its boundary,
    /// try its ancestors, never a covered sibling underneath it.
    fn scroll_target(&self, point: Point, delta: Point) -> Option<ElementId> {
        let mut target = self.paint_order.iter().rev().copied().find(|id| {
            self.entries
                .get(id)
                .is_some_and(|e| e.rect.contains(point) && e.clip.contains(point))
        });
        while let Some(id) = target {
            let entry = self.entries.get(&id)?;
            if self.scrolled_offset(id, delta).is_some_and(|next| {
                entry
                    .scroll_geometry()
                    .is_some_and(|(_, _, current, _)| next != current)
            }) {
                return Some(id);
            }
            target = entry.parent;
        }
        None
    }

    fn scrolled_offset(&self, id: ElementId, delta: Point) -> Option<Point> {
        let entry = self.entries.get(&id)?;
        let (axis, _, mut next, max) = entry.scroll_geometry()?;
        if matches!(axis, Axis::Horizontal | Axis::Both) {
            next.x = (next.x - delta.x).clamp(0.0, max.x);
        }
        if matches!(axis, Axis::Vertical | Axis::Both) {
            next.y = (next.y - delta.y).clamp(0.0, max.y);
        }
        Some(next)
    }

    fn scroll_target_by(&mut self, id: ElementId, delta: Point) -> bool {
        let Some(next) = self.scrolled_offset(id, delta) else {
            return false;
        };
        let entry = self.entries.get_mut(&id).unwrap();
        let changed = entry
            .scroll_geometry()
            .is_some_and(|(_, _, current, _)| current != next);
        entry.set_scroll(next);
        self.scrolling.reveal(id, Instant::now());
        changed
    }

    /// Discrete wheels route each event. Precise gestures keep their target until
    /// the phase ends, even when the pointer crosses another pane. Winit merges
    /// native touch/momentum phases, so the next Started begins a new segment.
    pub fn scroll_event(
        &mut self,
        point: Point,
        delta: Point,
        phase: TouchPhase,
        precise: bool,
    ) -> bool {
        if !delta.x.is_finite() || !delta.y.is_finite() || self.scrolling.drag.is_some() {
            return false;
        }
        if !precise {
            self.scrolling.target = None;
            return self.scroll(point, delta);
        }
        let now = Instant::now();
        // Some platform/device combinations emit only Moved. Bound capture by
        // inactivity as well, so a later gesture can target a different pane.
        if phase == TouchPhase::Started
            || self
                .scrolling
                .last_event
                .is_some_and(|last| now.duration_since(last) > Duration::from_millis(250))
        {
            self.scrolling.target = None;
        }
        self.scrolling.last_event = Some(now);
        // Removed targets invalidate capture; a later delta can acquire a live one.
        if self
            .scrolling
            .target
            .is_some_and(|id| !self.entries.contains_key(&id))
        {
            self.scrolling.target = None;
        }
        if self.scrolling.target.is_none() {
            self.scrolling.target = self.scroll_target(point, delta);
        }
        let changed = self
            .scrolling
            .target
            .is_some_and(|id| self.scroll_target_by(id, delta));
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.scrolling.target = None;
        }
        changed
    }

    pub fn scroll(&mut self, point: Point, delta: Point) -> bool {
        self.scroll_target(point, delta)
            .is_some_and(|id| self.scroll_target_by(id, delta))
    }

    pub fn cancel_scroll(&mut self) {
        self.scrolling.target = None;
        self.scrolling.last_event = None;
        self.scrolling.drag = None;
        self.scrolling.hover = None;
        for presentation in self.scrolling.presentation.values_mut() {
            presentation.hold(false, Instant::now());
        }
    }

    /// Called by the window's event-loop deadline scheduler. A hidden scrollbar
    /// never keeps an idle app repainting. These deadlines do not run app effects.
    pub fn scrollbar_deadline(&self) -> Option<Instant> {
        self.scrolling.deadline
    }

    pub fn scrollbar_pointer(&mut self, point: Option<Point>) -> bool {
        self.update_scrollbar_hover(point, true)
    }

    /// Layout can move a track away from the pointer, but moving a hidden track
    /// underneath a stationary pointer is not user interaction. Only retain or
    /// release an existing hover; pointer events may discover a new track.
    pub fn reconcile_scrollbar_hover(&mut self, point: Option<Point>) -> bool {
        self.update_scrollbar_hover(point, false)
    }

    fn update_scrollbar_hover(&mut self, point: Option<Point>, discover: bool) -> bool {
        let hover = point
            .and_then(|point| {
                self.scrolling
                    .bars
                    .iter()
                    .rev()
                    .find(|bar| self.bar_contains(bar, point))
                    .map(|bar| (bar.target, bar.vertical))
            })
            .filter(|key| discover || self.scrolling.hover == Some(*key));
        let changed = hover != self.scrolling.hover;
        self.scrolling.hover = hover;
        if changed {
            let now = Instant::now();
            for (key, presentation) in &mut self.scrolling.presentation {
                let dragging = self
                    .scrolling
                    .drag
                    .is_some_and(|drag| (drag.bar.target, drag.bar.vertical) == *key);
                presentation.hold(hover == Some(*key) || dragging, now);
            }
        }
        changed
    }

    /// Reveal even at a scroll boundary, where no offset can change. Follow the
    /// visible surface's ancestry rather than showing covered sibling scrollbars.
    pub fn reveal_scrollbars_at(&mut self, point: Point) -> bool {
        let mut target = self.paint_order.iter().rev().copied().find(|id| {
            self.entries
                .get(id)
                .is_some_and(|entry| entry.rect.contains(point) && entry.clip.contains(point))
        });
        while let Some(id) = target {
            if self.scrolling.reveal(id, Instant::now()) {
                return true;
            }
            target = self.entries.get(&id).and_then(|entry| entry.parent);
        }
        false
    }

    fn bar_contains(&self, bar: &Scrollbar, point: Point) -> bool {
        if !self.entries.contains_key(&bar.target)
            || !bar.track.contains(point)
            || !bar.clip.contains(point)
        {
            return false;
        }
        let mut top = self.paint_order.iter().rev().copied().find(|id| {
            self.entries
                .get(id)
                .is_some_and(|e| e.rect.contains(point) && e.clip.contains(point))
        });
        while let Some(id) = top {
            if id == bar.target {
                return true;
            }
            top = self.entries.get(&id).and_then(|e| e.parent);
        }
        false
    }

    pub fn scrollbar_hit(&self, point: Point) -> bool {
        self.scrolling
            .bars
            .iter()
            .rev()
            .any(|bar| self.bar_contains(bar, point))
    }

    /// The scrollbar consumes the press before controls underneath see it. Track
    /// clicks center the thumb; drags retain the original grab point in the thumb.
    pub fn scrollbar_press(&mut self, point: Point) -> bool {
        let Some(bar) = self
            .scrolling
            .bars
            .iter()
            .rev()
            .copied()
            .find(|bar| self.bar_contains(bar, point))
        else {
            return false;
        };
        let p = if bar.vertical { point.y } else { point.x };
        let start = if bar.vertical {
            bar.thumb.y
        } else {
            bar.thumb.x
        };
        let length = if bar.vertical {
            bar.thumb.height
        } else {
            bar.thumb.width
        };
        let grab = if bar.thumb.contains(point) {
            p - start
        } else {
            length / 2.0
        };
        self.scrolling.target = None;
        self.scrolling.drag = Some(Drag {
            bar,
            grab,
            pointer: point,
        });
        if let Some(presentation) = self
            .scrolling
            .presentation
            .get_mut(&(bar.target, bar.vertical))
        {
            presentation.hold(true, Instant::now());
        }
        self.scrollbar_drag(point);
        true
    }

    pub fn scrollbar_drag(&mut self, point: Point) -> bool {
        let Some(drag) = self.scrolling.drag else {
            return false;
        };
        // Re-read geometry after resize/content changes, preserving capture by ID.
        let Some(bar) = self
            .scrolling
            .bars
            .iter()
            .find(|bar| bar.target == drag.bar.target && bar.vertical == drag.bar.vertical)
            .copied()
        else {
            self.scrolling.drag = None;
            return false;
        };
        let Some(entry) = self.entries.get_mut(&bar.target) else {
            self.scrolling.drag = None;
            return false;
        };
        let (p, start, travel) = if bar.vertical {
            (point.y, bar.track.y, bar.track.height - bar.thumb.height)
        } else {
            (point.x, bar.track.x, bar.track.width - bar.thumb.width)
        };
        let offset = ((p - start - drag.grab) / travel.max(1.0)).clamp(0.0, 1.0) * bar.max;
        let Some((_, _, mut current, _)) = entry.scroll_geometry() else {
            return false;
        };
        if bar.vertical {
            current.y = offset;
        } else {
            current.x = offset;
        }
        entry.set_scroll(current);
        self.scrolling.drag.as_mut().unwrap().pointer = point;
        true
    }

    pub fn scrollbar_release(&mut self) -> bool {
        let Some(drag) = self.scrolling.drag.take() else {
            return false;
        };
        let key = (drag.bar.target, drag.bar.vertical);
        if let Some(presentation) = self.scrolling.presentation.get_mut(&key) {
            presentation.reveal(Instant::now());
            presentation.hold(self.scrolling.hover == Some(key), Instant::now());
        }
        true
    }

    /// Overlay scrollbars do not consume layout space. Paint after descendants,
    /// within the container's existing clip/opacity layer, and retain that order
    /// for pointer hit testing. No bar is emitted for subpixel overflow.
    pub(super) fn paint_scrollbars(&mut self, id: ElementId, frame: &mut Frame) {
        let entry = &self.entries[&id];
        let Some((axis, r, scroll, max)) = entry.scroll_geometry() else {
            return;
        };
        let vertical = matches!(axis, Axis::Vertical | Axis::Both) && max.y > 0.5;
        let horizontal = matches!(axis, Axis::Horizontal | Axis::Both) && max.x > 0.5;
        for is_vertical in [true, false] {
            if !(if is_vertical { vertical } else { horizontal }) {
                continue;
            }
            let track = if is_vertical {
                Rect {
                    x: r.x + r.width - 12.0,
                    y: r.y + 3.0,
                    width: 9.0,
                    height: (r.height - 6.0 - if horizontal { 12.0 } else { 0.0 }).max(0.0),
                }
            } else {
                Rect {
                    x: r.x + 3.0,
                    y: r.y + r.height - 12.0,
                    width: (r.width - 6.0 - if vertical { 12.0 } else { 0.0 }).max(0.0),
                    height: 9.0,
                }
            };
            let (length, viewport, max, offset) = if is_vertical {
                (track.height, r.height, max.y, scroll.y)
            } else {
                (track.width, r.width, max.x, scroll.x)
            };
            if length <= 0.0 {
                continue;
            }
            let target_length = (length * viewport / (viewport + max)).max(24.0).min(length);
            let key = (id, is_vertical);
            let now = self.scrolling.frame_time.unwrap();
            let active = self
                .scrolling
                .drag
                .is_some_and(|d| (d.bar.target, d.bar.vertical) == key);
            let presentation = self.scrolling.presentation.entry(key).or_default();
            presentation.hold(active || self.scrolling.hover == Some(key), now);
            presentation.observe_offset(offset, now, !self.scrolling.resized);
            let thumb_length = presentation.length(target_length, length, now);
            let opacity = presentation.opacity(now);
            let position = (length - thumb_length) * (offset / max).clamp(0.0, 1.0);
            let thumb = if is_vertical {
                Rect {
                    y: track.y + position,
                    height: thumb_length,
                    ..track
                }
            } else {
                Rect {
                    x: track.x + position,
                    width: thumb_length,
                    ..track
                }
            };
            if opacity > 0.0 {
                frame.commands.push(DrawCommand::Fill {
                    rect: track,
                    radius: 4.5.into(),
                    color: Color(128, 128, 128, (32.0 * opacity).round() as u8),
                });
                frame.commands.push(DrawCommand::Fill {
                    rect: thumb,
                    radius: 4.5.into(),
                    color: Color(
                        128,
                        128,
                        128,
                        ((if active { 230.0 } else { 155.0 }) * opacity).round() as u8,
                    ),
                });
            }
            // Rebase the grab point when layout or the displayed thumb length
            // changes. A stationary pointer must not jump the content next move.
            if let Some(drag) = &mut self.scrolling.drag {
                if (drag.bar.target, drag.bar.vertical) == key {
                    if drag.bar.track != track
                        || drag.bar.max != max
                        || drag.bar.thumb.width != thumb.width
                        || drag.bar.thumb.height != thumb.height
                    {
                        drag.grab = if is_vertical {
                            drag.pointer.y - thumb.y
                        } else {
                            drag.pointer.x - thumb.x
                        };
                    }
                    drag.bar = Scrollbar {
                        target: id,
                        vertical: is_vertical,
                        track,
                        thumb,
                        clip: entry.clip,
                        max,
                    };
                }
            }
            self.scrolling.bars.push(Scrollbar {
                target: id,
                vertical: is_vertical,
                track,
                thumb,
                clip: entry.clip,
                max,
            });
        }
    }
}

#[cfg(test)]
mod tests;

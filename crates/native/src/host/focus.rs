//! Focus traversal uses retained topology; reveal uses the current layout rather
//! than last frame's rectangles. Neither operation evaluates application views.

use super::Host;
use crate::renderer::Rect;
use refract_core::{Axis, ElementId, Layout, ScrollOffset};

impl Host {
    pub(super) fn element_order(&self) -> Vec<ElementId> {
        let mut order = Vec::with_capacity(self.entries.len());
        let mut stack: Vec<_> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(self.entries[&id].children.iter().rev().copied());
        }
        order
    }

    pub(super) fn focus_recovery_order(&self, focused: ElementId) -> Vec<ElementId> {
        let order = self.element_order();
        let Some(index) = order.iter().position(|id| *id == focused) else {
            return Vec::new();
        };
        order[index + 1..]
            .iter()
            .chain(order[..index].iter().rev())
            .copied()
            .collect()
    }

    pub(super) fn reveal_focused_control(&mut self) -> crate::Result<()> {
        let Some(entry) = self.focus.and_then(|id| self.entries.get(&id)) else {
            return Ok(());
        };
        let layout = self.tree.layout(entry.node)?;
        let ring = entry.element.style.focus_ring.unwrap_or_default();
        let pad = if ring.width > 0.0 {
            ring.width + ring.offset
        } else {
            0.0
        };
        let mut target = Rect {
            x: layout.location.x - pad,
            y: layout.location.y - pad,
            width: layout.size.width + 2.0 * pad,
            height: layout.size.height + 2.0 * pad,
        };
        let mut parent = entry.parent;
        // Each iteration expresses the visible part of the target in its
        // parent's coordinates, then walks outward through nested viewports.
        while let Some(id) = parent {
            let entry = self.entries.get_mut(&id).unwrap();
            let layout = self.tree.layout(entry.node)?;
            let viewport = Rect {
                x: layout.border.left,
                y: layout.border.top,
                width: (layout.size.width - layout.border.left - layout.border.right).max(0.0),
                height: (layout.size.height - layout.border.top - layout.border.bottom).max(0.0),
            };
            target.x -= entry.scroll.x;
            target.y -= entry.scroll.y;
            if let Layout::Scroll(axis) = entry.element.style.layout {
                let old = entry.scroll;
                if matches!(axis, Axis::Horizontal | Axis::Both) && viewport.width > 0.0 {
                    entry.scroll.x = (old.x
                        + nearest_delta(target.x, target.width, viewport.x, viewport.width))
                    .clamp(0.0, entry.scroll_max.x);
                }
                if matches!(axis, Axis::Vertical | Axis::Both) && viewport.height > 0.0 {
                    entry.scroll.y = (old.y
                        + nearest_delta(target.y, target.height, viewport.y, viewport.height))
                    .clamp(0.0, entry.scroll_max.y);
                }
                target.x -= entry.scroll.x - old.x;
                target.y -= entry.scroll.y - old.y;
                entry.scroll_metrics.offset = ScrollOffset {
                    x: f64::from(entry.scroll.x),
                    y: f64::from(entry.scroll.y),
                };
                entry.scroll_tracker.record(entry.scroll_metrics);
                target = target.intersect(viewport);
            } else if entry.element.style.clipped {
                target = target.intersect(viewport);
            }
            target.x += layout.location.x;
            target.y += layout.location.y;
            parent = entry.parent;
        }
        Ok(())
    }
}

/// Align the nearest edge. An oversized control spanning the viewport already
/// has a useful visible region and must not oscillate between its two edges.
fn nearest_delta(start: f32, size: f32, viewport: f32, available: f32) -> f32 {
    let end = start + size;
    let limit = viewport + available;
    if start < viewport && end > limit {
        0.0
    } else if start < viewport {
        if size > available {
            end - limit
        } else {
            start - viewport
        }
    } else if end > limit {
        if size > available {
            start - viewport
        } else {
            end - limit
        }
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::{prelude::*, App};

    fn flush(app: &mut App, host: &mut Host) {
        if let Some(pending) = app.prepare().unwrap() {
            host.apply(&pending.update).unwrap();
            app.commit(pending).unwrap();
        }
    }

    fn frame(host: &mut Host) {
        host.frame(600.0, 500.0, 1.0, Color::WHITE).unwrap();
    }

    fn named(host: &Host, name: &str) -> ElementId {
        host.entries
            .iter()
            .find(|(_, e)| e.element.label.as_deref() == Some(name))
            .unwrap()
            .0
            .to_owned()
    }

    #[test]
    fn tab_reveals_nested_viewports_and_reports_scroll_without_relayout() {
        let mut app = App::new(
            scroll_view(
                v_stack()
                    .child(view().height(240.0))
                    .child(
                        scroll_view(
                            h_stack().child(view().width(360.0)).child(
                                text("target")
                                    .label("target")
                                    .focusable(true)
                                    .width(60.0)
                                    .height(30.0),
                            ),
                        )
                        .axis(Axis::Horizontal)
                        .width(180.0)
                        .height(60.0)
                        .label("inner")
                        .border(Border::all(3.0, Color::BLACK))
                        .on_scroll(|_, _| {}),
                    )
                    .child(view().height(240.0)),
            )
            .width(220.0)
            .height(120.0)
            .label("outer")
            .border(Border::all(4.0, Color::BLACK))
            .on_scroll(|_, _| {}),
        );
        let mut host = Host::new().unwrap();
        flush(&mut app, &mut host);
        frame(&mut host);
        host.take_scroll_events();
        let before = host.stats();
        host.focus_next(false);
        frame(&mut host);
        let target = &host.entries[&named(&host, "target")];
        assert_eq!(target.rect.intersect(target.clip), target.rect);
        assert!(host.entries[&named(&host, "inner")].scroll.x > 0.0);
        assert!(host.entries[&named(&host, "outer")].scroll.y > 0.0);
        assert_eq!(host.take_scroll_events().len(), 2);
        assert_eq!(host.stats().layout_passes, before.layout_passes);
        assert_eq!(host.stats().text_shapes, before.text_shapes);
        let offsets: Vec<_> = host.entries.values().map(|e| e.scroll).collect();
        frame(&mut host);
        assert_eq!(
            host.entries.values().map(|e| e.scroll).collect::<Vec<_>>(),
            offsets
        );
        assert!(host.take_scroll_events().is_empty());
    }

    #[test]
    fn reverse_tab_reveals_previous_control_but_manual_scrolling_stays_put() {
        let mut app = App::new(
            scroll_view(
                v_stack()
                    .child(text("first").label("first").focusable(true).height(30.0))
                    .child(view().height(300.0))
                    .child(text("disabled").focusable(true).disabled(true))
                    .child(text("skipped").focusable(false))
                    .child(text("last").label("last").focusable(true).height(30.0)),
            )
            .width(200.0)
            .height(100.0)
            .label("pane"),
        );
        let mut host = Host::new().unwrap();
        flush(&mut app, &mut host);
        host.focus_next(false);
        host.focus_next(false);
        frame(&mut host);
        assert_eq!(host.focus, Some(named(&host, "last")));
        let pane = named(&host, "pane");
        assert!(host.entries[&pane].scroll.y > 0.0);
        host.focus_next(true);
        frame(&mut host);
        assert_eq!(host.focus, Some(named(&host, "first")));
        assert_eq!(host.entries[&pane].scroll.y, 0.0);
        let rect = host.entries[&pane].rect;
        assert!(host.scroll(
            crate::renderer::Point {
                x: rect.x + 10.0,
                y: rect.y + 10.0
            },
            crate::renderer::Point { x: 0.0, y: -50.0 }
        ));
        frame(&mut host);
        assert_eq!(host.entries[&pane].scroll.y, 50.0);
        frame(&mut host);
        assert_eq!(host.entries[&pane].scroll.y, 50.0);
    }

    #[test]
    fn navigation_uses_new_topology_before_the_next_paint() {
        let form = |reverse| {
            v_stack().children(
                (if reverse {
                    vec!["c", "b", "a"]
                } else {
                    vec!["a", "b", "c"]
                })
                .into_iter()
                .map(|name| text(name).label(name).focusable(true).key(name)),
            )
        };
        let mut app = App::new(form(false));
        let mut host = Host::new().unwrap();
        flush(&mut app, &mut host);
        // No frame has been produced yet.
        host.focus_next(false);
        assert_eq!(host.focus, Some(named(&host, "a")));
        app.set_view(form(true));
        flush(&mut app, &mut host);
        host.focus_next(false);
        assert_eq!(host.focus, Some(named(&host, "c")));
        host.focus_next(true);
        assert_eq!(host.focus, Some(named(&host, "a")));
    }

    #[test]
    fn focus_recovers_forward_then_backward_and_clears_when_empty() {
        let form = |removed: Option<&str>, disabled: Option<&str>, skipped: Option<&str>| {
            v_stack().children(
                ["a", "b", "c"]
                    .into_iter()
                    .filter(move |name| Some(*name) != removed)
                    .map(move |name| {
                        text(name)
                            .label(name)
                            .focusable(Some(name) != skipped)
                            .disabled(Some(name) == disabled)
                            .key(name)
                    }),
            )
        };
        for mode in 0..3 {
            let mut app = App::new(form(None, None, None));
            let mut host = Host::new().unwrap();
            flush(&mut app, &mut host);
            host.focus_next(false);
            host.focus_next(false);
            let b = named(&host, "b");
            assert_eq!(host.focus, Some(b));
            host.take_input_events();
            app.set_view(form(
                (mode == 0).then_some("b"),
                (mode == 1).then_some("b"),
                (mode == 2).then_some("b"),
            ));
            flush(&mut app, &mut host);
            let c = named(&host, "c");
            assert_eq!(host.focus, Some(c));
            assert!(host
                .take_input_events()
                .iter()
                .any(|(id, e)| *id == c && matches!(e, refract_core::Event::Focus(true))));
            app.set_view(form(Some("c"), Some("b"), None));
            flush(&mut app, &mut host);
            assert_eq!(host.focus, Some(named(&host, "a")));
            app.set_view(Empty);
            flush(&mut app, &mut host);
            assert_eq!(host.focus, None);
        }
    }

    #[test]
    fn nearest_edge_handles_oversized_and_already_visible_controls() {
        assert_eq!(nearest_delta(20.0, 30.0, 0.0, 100.0), 0.0);
        assert_eq!(nearest_delta(120.0, 30.0, 0.0, 100.0), 50.0);
        assert_eq!(nearest_delta(-10.0, 30.0, 0.0, 100.0), -10.0);
        assert_eq!(nearest_delta(-10.0, 200.0, 0.0, 100.0), 0.0);
        assert_eq!(nearest_delta(120.0, 200.0, 0.0, 100.0), 120.0);
        assert_eq!(nearest_delta(-220.0, 200.0, 0.0, 100.0), -120.0);
    }
}

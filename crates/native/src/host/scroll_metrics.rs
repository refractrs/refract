//! Resolve scroll geometry after Taffy and expose snapshots after a host frame.
//! Input may change offsets many times between frames; notifications report the
//! net movement once, without feeding those offsets through the reactive graph.

use super::Host;
use refract_core::{ElementId, Event, Layout, ScrollMetrics, ScrollOffset, ScrollSize};

impl Host {
    pub(super) fn resolve_scroll_layout(&mut self) -> crate::Result<()> {
        for entry in self.entries.values_mut() {
            let Layout::Scroll(axis) = entry.element.style.layout else {
                continue;
            };
            let layout = self.tree.layout(entry.node)?;
            let width = (layout.size.width - layout.border.left - layout.border.right).max(0.0);
            let height = (layout.size.height - layout.border.top - layout.border.bottom).max(0.0);
            let content_width = layout.scrollable_overflow_rect.right.max(width);
            let content_height = layout.scrollable_overflow_rect.bottom.max(height);
            let metrics = ScrollMetrics {
                offset: ScrollOffset {
                    x: entry.scroll.x as f64,
                    y: entry.scroll.y as f64,
                },
                viewport_size: ScrollSize {
                    width: width as f64,
                    height: height as f64,
                },
                content_size: ScrollSize {
                    width: content_width as f64,
                    height: content_height as f64,
                },
            };
            let metrics = entry.scroll_tracker.resolve(
                entry.element.scroll.unwrap_or_default(),
                axis,
                metrics,
            );
            let max = metrics.max_offset();
            entry.scroll.x = metrics.offset.x as f32;
            entry.scroll.y = metrics.offset.y as f32;
            entry.scroll_max.x = max.x as f32;
            entry.scroll_max.y = max.y as f32;
            entry.scroll_tracker.record(metrics);
            entry.scroll_metrics = metrics;
        }
        Ok(())
    }

    /// Drain after a frame, outside layout/paint. Callbacks may safely mutate the
    /// app; their resulting patches will be prepared for the next frame.
    pub fn take_scroll_events(&mut self) -> Vec<(ElementId, Event)> {
        self.paint_order
            .iter()
            .filter_map(|id| {
                let entry = self.entries.get_mut(id)?;
                let options = entry.element.scroll?;
                entry
                    .scroll_tracker
                    .notification(entry.scroll_metrics, options.observe)
                    .map(|event| (*id, Event::Scroll(event)))
            })
            .collect()
    }
}

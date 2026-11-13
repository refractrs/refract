//! Portable scroll configuration and geometry. Positions use logical pixels
//! (CSS pixels on the web), with positive x/right and y/down coordinates.

use crate::{ActionContext, Axis, Description, Element, ElementKind, Event, Handler, Layout, View};

/// A scroll position or displacement in logical pixels; positive values point right and down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollOffset {
    pub x: f64,
    pub y: f64,
}

/// The width and height of a scroll viewport or its content, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollSize {
    pub width: f64,
    pub height: f64,
}

/// A snapshot of the scroll viewport, including padding but excluding borders.
/// Content size is at least the viewport size. Offsets exclude rubber-banding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollMetrics {
    pub offset: ScrollOffset,
    pub viewport_size: ScrollSize,
    pub content_size: ScrollSize,
}

impl ScrollMetrics {
    pub fn max_offset(self) -> ScrollOffset {
        ScrollOffset {
            x: (self.content_size.width - self.viewport_size.width).max(0.0),
            y: (self.content_size.height - self.viewport_size.height).max(0.0),
        }
    }

    pub fn distance_to_end(self) -> ScrollOffset {
        let max = self.max_offset();
        ScrollOffset {
            x: (max.x - self.offset.x).max(0.0),
            y: (max.y - self.offset.y).max(0.0),
        }
    }

    /// With `Both`, both axes must be at their end. Non-overflowing axes qualify.
    pub fn is_at_end(self, axis: Axis, tolerance: f64) -> bool {
        let distance = self.distance_to_end();
        let tolerance = tolerance.max(0.0);
        match axis {
            Axis::Horizontal => distance.x <= tolerance,
            Axis::Vertical => distance.y <= tolerance,
            Axis::Both => distance.x <= tolerance && distance.y <= tolerance,
        }
    }
}

/// Delivered at most once per adapter frame, directly to this scroll view (no
/// bubbling). The first snapshot has zero delta. Later snapshots report changes
/// to position OR geometry, so viewport resize/content growth can have zero delta.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollEvent {
    pub metrics: ScrollMetrics,
    /// Net actual movement since the previous notification, not wheel intent.
    pub delta: ScrollOffset,
}

/// The initial position of a scroll view along its enabled axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollPosition {
    /// Start at the beginning of the content.
    #[default]
    Start,
    /// Start at the end of the content.
    End,
}

/// Whether a scroll view follows its content's end when geometry changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollFollow {
    /// Preserve the current position, subject to the scrollable bounds.
    #[default]
    None,
    /// Follow geometry changes while within two logical pixels of the old end.
    /// Each enabled axis follows independently. Scrolling away suspends following.
    EndWhenPinned,
}

/// Behavioral properties of a scroll element, separate from its visual style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollOptions {
    pub initial_position: ScrollPosition,
    pub follow: ScrollFollow,
    /// Whether the adapter should deliver snapshots to the retained handler.
    pub observe: bool,
}

/// A scroll container with type-preserving style and behavior modifiers.
#[derive(Clone)]
pub struct ScrollView {
    element: Element,
    content: crate::AnyView,
    handler: Option<Handler>,
}

/// Create a vertical scroll viewport. Use `.axis(...)` to change its direction.
pub fn scroll_view(content: impl View) -> ScrollView {
    let mut element = Element::new(ElementKind::Box);
    element.style.layout = Layout::Scroll(Axis::Vertical);
    element.scroll = Some(ScrollOptions::default());
    ScrollView {
        element,
        content: content.into_any(),
        handler: None,
    }
}

impl crate::view::modifiers::ElementBuilder for ScrollView {
    fn element_mut(&mut self) -> &mut crate::Element {
        &mut self.element
    }
    fn handler_mut(&mut self) -> &mut Option<Handler> { &mut self.handler }
}

impl ScrollView {
    crate::view::modifiers::element_modifiers!(pub Self, crate::view::modifiers::configure);
    crate::view::modifiers::event_modifiers!(pub Self, crate::view::modifiers::configure_handler);
    crate::view::modifiers::focus_modifiers!(pub Self, crate::view::modifiers::configure_focus);

    /// Choose the scrolling direction. Defaults to `Axis::Vertical`.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.element.style.layout = Layout::Scroll(axis);
        self
    }

    /// Applied once when this mounted scroll view first has a nonzero viewport.
    pub fn initial_position(mut self, position: ScrollPosition) -> Self {
        self.element.scroll.as_mut().unwrap().initial_position = position;
        self
    }

    pub fn follow(mut self, policy: ScrollFollow) -> Self {
        self.element.scroll.as_mut().unwrap().follow = policy;
        self
    }

    /// Add pointer/keyboard activation while preserving scroll configuration.
    pub fn on_press(mut self, action: impl Fn(&mut ActionContext<'_>) + 'static) -> Self {
        crate::element::add_press_handler(&mut self.element, &mut self.handler, action);
        self
    }

    /// Observe actual scrolling and geometry changes from either runtime.
    /// Reading an event does not invalidate views; writing graph state may do so.
    /// Callbacks compose in registration order and preserve activation handlers.
    pub fn on_scroll(
        mut self,
        action: impl Fn(&mut ActionContext<'_>, &ScrollEvent) + 'static,
    ) -> Self {
        self.element.scroll.as_mut().unwrap().observe = true;
        let previous = self.handler.take();
        let focus = previous.as_ref().and_then(|h| h.focus.clone());
        self.handler = Some(Handler::new(move |cx, event| {
            if let Some(previous) = &previous {
                previous(cx, event);
            }
            if let Event::Scroll(event) = event {
                action(cx, event);
            }
        }));
        if let Some(handler) = &mut self.handler {
            handler.focus = focus;
        }
        self
    }
}

impl View for ScrollView {
    fn describe(&self) -> Option<Description> {
        Some(Description::Element(
            Box::new(self.element.clone()),
            vec![self.content.clone()],
            self.handler.clone(),
        ))
    }
}

/// Shared adapter bookkeeping; never reads or writes the application graph.
/// Kept separate from scroll input, which remains platform-owned.
#[doc(hidden)]
#[derive(Default)]
pub struct ScrollTracker {
    previous: Option<ScrollMetrics>,
    notified: Option<ScrollMetrics>,
}

impl ScrollTracker {
    /// Resolve initial/follow policy after layout, before painting or notifying.
    pub fn resolve(
        &self,
        options: ScrollOptions,
        axis: Axis,
        mut metrics: ScrollMetrics,
    ) -> ScrollMetrics {
        let max = metrics.max_offset();
        let old_max = self.previous.map(ScrollMetrics::max_offset);
        let initial_end =
            self.previous.is_none() && options.initial_position == ScrollPosition::End;
        let follow = options.follow == ScrollFollow::EndWhenPinned;
        if matches!(axis, Axis::Horizontal | Axis::Both) {
            if initial_end || (follow && old_max.is_some_and(|old| old.x - metrics.offset.x <= 2.0))
            {
                metrics.offset.x = max.x;
            }
            metrics.offset.x = metrics.offset.x.clamp(0.0, max.x);
        } else {
            metrics.offset.x = 0.0;
        }
        if matches!(axis, Axis::Vertical | Axis::Both) {
            if initial_end || (follow && old_max.is_some_and(|old| old.y - metrics.offset.y <= 2.0))
            {
                metrics.offset.y = max.y;
            }
            metrics.offset.y = metrics.offset.y.clamp(0.0, max.y);
        } else {
            metrics.offset.y = 0.0;
        }
        metrics
    }

    pub fn record(&mut self, metrics: ScrollMetrics) {
        // Hidden/zero-size mounts should still receive their initial position
        // when they become visible, rather than consuming it against no layout.
        if metrics.viewport_size.width > 0.0 && metrics.viewport_size.height > 0.0 {
            self.previous = Some(metrics);
        }
    }

    pub fn notification(&mut self, metrics: ScrollMetrics, enabled: bool) -> Option<ScrollEvent> {
        if !enabled {
            self.notified = None;
            return None;
        }
        if self.notified == Some(metrics) {
            return None;
        }
        let previous = self.notified.replace(metrics).unwrap_or(metrics);
        Some(ScrollEvent {
            metrics,
            delta: ScrollOffset {
                x: metrics.offset.x - previous.offset.x,
                y: metrics.offset.y - previous.offset.y,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(x: f64, y: f64, width: f64, height: f64) -> ScrollMetrics {
        ScrollMetrics {
            offset: ScrollOffset { x, y },
            viewport_size: ScrollSize {
                width: 100.0,
                height: 100.0,
            },
            content_size: ScrollSize { width, height },
        }
    }

    #[test]
    fn initial_position_and_following_are_independent_and_axis_aware() {
        let mut tracker = ScrollTracker::default();
        let mut options = ScrollOptions {
            initial_position: ScrollPosition::End,
            ..Default::default()
        };
        let initial = tracker.resolve(options, Axis::Both, metrics(0.0, 0.0, 300.0, 500.0));
        assert_eq!(initial.offset, ScrollOffset { x: 200.0, y: 400.0 });
        tracker.record(initial);
        let grown = tracker.resolve(options, Axis::Both, metrics(200.0, 400.0, 400.0, 600.0));
        assert_eq!(
            grown.offset, initial.offset,
            "initial End doesn't imply follow"
        );
        options.follow = ScrollFollow::EndWhenPinned;
        let moved = tracker.resolve(options, Axis::Both, metrics(120.5, 400.0, 400.0, 600.0));
        assert_eq!(
            moved.offset,
            ScrollOffset { x: 120.5, y: 500.0 },
            "axes follow independently"
        );
        tracker.record(moved);
        let resized = tracker.resolve(
            options,
            Axis::Both,
            ScrollMetrics {
                viewport_size: ScrollSize {
                    width: 200.0,
                    height: 200.0,
                },
                ..moved
            },
        );
        assert_eq!(resized.offset, ScrollOffset { x: 120.5, y: 400.0 });
        let shrunk = tracker.resolve(options, Axis::Both, metrics(120.5, 400.0, 20.0, 20.0));
        assert_eq!(shrunk.offset, ScrollOffset::default());
    }

    #[test]
    fn snapshots_coalesce_actual_movement_and_report_geometry_without_movement() {
        let mut tracker = ScrollTracker::default();
        let start = metrics(0.0, 0.0, 300.0, 500.0);
        assert_eq!(
            tracker.notification(start, true).unwrap().delta,
            ScrollOffset::default()
        );
        assert!(tracker.notification(start, true).is_none());
        tracker.record(metrics(0.0, 2.5, 300.0, 500.0));
        tracker.record(metrics(0.0, 12.25, 300.0, 500.0));
        let latest = metrics(0.0, 7.75, 300.0, 500.0);
        assert_eq!(tracker.notification(latest, true).unwrap().delta.y, 7.75);
        let resized = ScrollMetrics {
            viewport_size: ScrollSize {
                width: 150.0,
                height: 100.0,
            },
            ..latest
        };
        assert_eq!(
            tracker.notification(resized, true).unwrap().delta,
            ScrollOffset::default()
        );
        assert!(tracker.notification(resized, false).is_none());
        assert_eq!(
            tracker.notification(resized, true).unwrap().delta,
            ScrollOffset::default()
        );
        assert!(metrics(0.0, 398.5, 300.0, 500.0).is_at_end(Axis::Vertical, 2.0));
        assert!(!metrics(0.0, 398.5, 300.0, 500.0).is_at_end(Axis::Both, 2.0));
    }

    #[test]
    fn zero_size_layout_does_not_consume_initial_position() {
        let mut tracker = ScrollTracker::default();
        tracker.record(ScrollMetrics::default());
        let options = ScrollOptions {
            initial_position: ScrollPosition::End,
            ..Default::default()
        };
        let visible = tracker.resolve(options, Axis::Vertical, metrics(0.0, 0.0, 100.0, 500.0));
        assert_eq!(visible.offset.y, 400.0);
    }
}

//! Time-based scrollbar presentation, independent of layout and input routing.
//! Geometry changes retarget from the currently displayed length. Visibility
//! deadlines let the window sleep between interaction and the short fade.

use std::time::{Duration, Instant};

// Keep geometry close to live resizing while smoothing abrupt content changes.
const RESIZE: Duration = Duration::from_millis(60);
const HOLD: Duration = Duration::from_millis(900);
const FADE: Duration = Duration::from_millis(220);
const FRAME: Duration = Duration::from_millis(16);

struct LengthTransition {
    from: f32,
    to: f32,
    start: Instant,
}

impl LengthTransition {
    fn value(&self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.start).as_secs_f32() / RESIZE.as_secs_f32())
            .min(1.0);
        // Smoothstep has no sudden velocity change at either endpoint.
        let t = t * t * (3.0 - 2.0 * t);
        self.from + (self.to - self.from) * t
    }
}

#[derive(Default)]
pub(super) struct Presentation {
    length: Option<LengthTransition>,
    last_activity: Option<Instant>,
    held: bool,
    offset: Option<f32>,
    pub seen: bool,
}

impl Presentation {
    pub fn reveal(&mut self, now: Instant) {
        self.last_activity = Some(now);
    }

    pub fn hold(&mut self, held: bool, now: Instant) {
        if self.held != held {
            self.reveal(now);
            self.held = held;
        }
    }

    /// Programmatic scrolling (caret reveal or follow-bottom) is activity too.
    /// Resize-driven clamping/following only updates the baseline. Repaints with
    /// the same offset must not extend the visibility deadline either.
    pub fn observe_offset(&mut self, offset: f32, now: Instant, reveal: bool) {
        if reveal
            && self
                .offset
                .is_some_and(|previous| (offset - previous).abs() > 0.01)
        {
            self.reveal(now);
        }
        self.offset = Some(offset);
    }

    pub fn opacity(&self, now: Instant) -> f32 {
        if self.held {
            return 1.0;
        }
        let Some(last) = self.last_activity else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(last);
        if elapsed <= HOLD {
            return 1.0;
        }
        (1.0 - (elapsed - HOLD).as_secs_f32() / FADE.as_secs_f32()).max(0.0)
    }

    pub fn length(&mut self, target: f32, track: f32, now: Instant) -> f32 {
        self.seen = true;
        let current = self
            .length
            .as_ref()
            .map_or(target, |transition| transition.value(now));
        if self
            .length
            .as_ref()
            .is_none_or(|transition| (transition.to - target).abs() > 0.01)
        {
            self.length = Some(LengthTransition {
                from: current,
                to: target,
                start: now,
            });
        }
        // The displayed thumb must always fit even during a large window shrink.
        current.min(track).max(0.0)
    }

    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        let opacity = self.opacity(now);
        let resize = self
            .length
            .as_ref()
            .filter(|length| {
                opacity > 0.0
                    && (length.from - length.to).abs() > 0.01
                    && now < length.start + RESIZE
            })
            .map(|length| (now + FRAME).min(length.start + RESIZE));
        let fade = if self.held {
            None
        } else {
            self.last_activity.and_then(|last| {
                let start = last + HOLD;
                let end = start + FADE;
                if now < start {
                    Some(start)
                } else if now < end {
                    Some((now + FRAME).min(end))
                } else {
                    None
                }
            })
        };
        resize.into_iter().chain(fade).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_has_a_deadline_and_stops_completely_when_hidden() {
        let now = Instant::now();
        let mut presentation = Presentation::default();
        assert_eq!(presentation.opacity(now), 0.0);
        assert_eq!(presentation.deadline(now), None);
        presentation.reveal(now);
        assert_eq!(presentation.deadline(now), Some(now + HOLD));
        assert_eq!(presentation.opacity(now + HOLD + FADE / 2), 0.5);
        assert_eq!(presentation.opacity(now + HOLD + FADE), 0.0);
        assert_eq!(presentation.deadline(now + HOLD + FADE), None);
        presentation.hold(true, now);
        assert_eq!(presentation.opacity(now + HOLD + FADE), 1.0);
        presentation.hold(false, now + HOLD + FADE);
        assert_eq!(presentation.opacity(now + HOLD + FADE), 1.0);
    }

    #[test]
    fn resize_retargets_continuously_and_settles_to_the_proportional_length() {
        let now = Instant::now();
        let mut presentation = Presentation::default();
        assert_eq!(presentation.length(100.0, 400.0, now), 100.0);
        assert_eq!(presentation.length(200.0, 400.0, now), 100.0);
        let middle = presentation.length(200.0, 400.0, now + RESIZE / 2);
        assert_eq!(middle, 150.0);
        assert_eq!(presentation.length(80.0, 400.0, now + RESIZE / 2), middle);
        assert_eq!(presentation.length(80.0, 400.0, now + RESIZE * 2), 80.0);
        assert!(presentation.length(20.0, 30.0, now + RESIZE * 2) <= 30.0);
    }
}

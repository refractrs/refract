//! Portable, deterministic animation timing. Hosts supply monotonic elapsed time.

use std::time::Duration;

/// Timing for a change to an animatable property. State always holds its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animation {
    curve: Curve,
    delay: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Curve {
    Timed(Duration, Easing),
    Spring { response: f64, damping: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Easing {
    Linear,
    In,
    Out,
    InOut,
}

/// A scalar presentation sample. Velocity is in units per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationSample {
    pub value: f64,
    pub velocity: f64,
    pub finished: bool,
}

impl Animation {
    pub fn linear(duration: Duration) -> Self {
        Self::timed(duration, Easing::Linear)
    }
    pub fn ease_in(duration: Duration) -> Self {
        Self::timed(duration, Easing::In)
    }
    pub fn ease_out(duration: Duration) -> Self {
        Self::timed(duration, Easing::Out)
    }
    pub fn ease_in_out(duration: Duration) -> Self {
        Self::timed(duration, Easing::InOut)
    }

    fn timed(duration: Duration, easing: Easing) -> Self {
        Self {
            curve: Curve::Timed(duration, easing),
            delay: Duration::ZERO,
        }
    }

    /// A lightly damped spring with a half-second response.
    pub fn spring() -> Self {
        Self::spring_with(Duration::from_millis(500), 0.85)
    }

    /// Damping is a positive ratio: below one oscillates, one is critically damped.
    /// Response must be nonzero. Invalid parameters panic at construction.
    pub fn spring_with(response: Duration, damping: f64) -> Self {
        assert!(!response.is_zero(), "spring response must be positive");
        assert!(
            damping.is_finite() && damping > 0.0,
            "spring damping must be finite and positive"
        );
        Self {
            curve: Curve::Spring {
                response: response.as_secs_f64(),
                damping,
            },
            delay: Duration::ZERO,
        }
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn is_spring(self) -> bool {
        matches!(self.curve, Curve::Spring { .. })
    }

    /// Sample from the current displayed value, carrying velocity when retargeting a spring.
    /// Timed curves deliberately start with their own velocity. A zero duration snaps
    /// after its delay. Springs settle when displacement and velocity are negligible.
    pub fn sample(self, from: f64, to: f64, velocity: f64, elapsed: Duration) -> AnimationSample {
        if elapsed < self.delay {
            return AnimationSample {
                value: from,
                velocity: 0.0,
                finished: false,
            };
        }
        let t = (elapsed - self.delay).as_secs_f64();
        match self.curve {
            Curve::Timed(duration, easing) => {
                let duration = duration.as_secs_f64();
                if t >= duration {
                    return AnimationSample {
                        value: to,
                        velocity: 0.0,
                        finished: true,
                    };
                }
                let x = t / duration;
                let (p, v) = match easing {
                    Easing::Linear => (x, 1.0),
                    Easing::In => (x * x * x, 3.0 * x * x),
                    Easing::Out => (1.0 - (1.0 - x).powi(3), 3.0 * (1.0 - x).powi(2)),
                    Easing::InOut => (x * x * (3.0 - 2.0 * x), 6.0 * x * (1.0 - x)),
                };
                AnimationSample {
                    value: from + (to - from) * p,
                    velocity: (to - from) * v / duration,
                    finished: false,
                }
            }
            Curve::Spring { response, damping } => {
                let w = std::f64::consts::TAU / response;
                let a = from - to;
                let (x, v) = if (damping - 1.0).abs() < 1e-6 {
                    let b = velocity + w * a;
                    let decay = (-w * t).exp();
                    ((a + b * t) * decay, (b - w * (a + b * t)) * decay)
                } else if damping < 1.0 {
                    let d = w * damping;
                    let wd = w * (1.0 - damping * damping).sqrt();
                    let b = (velocity + d * a) / wd;
                    let (sin, cos) = (wd * t).sin_cos();
                    let x = a * cos + b * sin;
                    let decay = (-d * t).exp();
                    (x * decay, (wd * (b * cos - a * sin) - d * x) * decay)
                } else {
                    let root = (damping * damping - 1.0).sqrt();
                    let r1 = -w / (damping + root);
                    let r2 = -w * (damping + root);
                    let c1 = (velocity - r2 * a) / (r1 - r2);
                    let c2 = a - c1;
                    let e1 = (r1 * t).exp();
                    let e2 = (r2 * t).exp();
                    (c1 * e1 + c2 * e2, r1 * c1 * e1 + r2 * c2 * e2)
                };
                let tolerance = 0.0001 * a.abs().max(1.0);
                let finished = x.abs() <= tolerance && v.abs() <= tolerance * w;
                if finished {
                    AnimationSample {
                        value: to,
                        velocity: 0.0,
                        finished: true,
                    }
                } else {
                    AnimationSample {
                        value: to + x,
                        velocity: v,
                        finished: false,
                    }
                }
            }
        }
    }
}

impl Default for Animation {
    fn default() -> Self {
        Self::ease_in_out(Duration::from_millis(250))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timed_endpoints_delay_and_zero_duration() {
        let a = Animation::linear(Duration::from_secs(1)).delay(Duration::from_millis(100));
        assert_eq!(a.sample(2.0, 4.0, 0.0, Duration::ZERO).value, 2.0);
        assert_eq!(
            a.sample(2.0, 4.0, 0.0, Duration::from_millis(600)).value,
            3.0
        );
        assert!(a.sample(2.0, 4.0, 0.0, Duration::from_secs(2)).finished);
        assert_eq!(
            Animation::linear(Duration::ZERO)
                .sample(2.0, 4.0, 0.0, Duration::ZERO)
                .value,
            4.0
        );
    }
    #[test]
    fn spring_retargets_with_continuous_position_and_velocity() {
        for damping in [0.5, 1.0, 2.0] {
            let a = Animation::spring_with(Duration::from_millis(500), damping);
            let mid = a.sample(0.0, 100.0, 0.0, Duration::from_millis(120));
            let next = a.sample(mid.value, -20.0, mid.velocity, Duration::ZERO);
            assert!((mid.value - next.value).abs() < 1e-9);
            assert!((mid.velocity - next.velocity).abs() < 1e-9);
            assert!(
                a.sample(mid.value, -20.0, mid.velocity, Duration::from_secs(20))
                    .finished
            );
        }
    }
    #[test]
    fn easing_velocity_matches_finite_difference() {
        for a in [
            Animation::ease_in,
            Animation::ease_out,
            Animation::ease_in_out,
        ] {
            let a = a(Duration::from_secs(1));
            let x = a.sample(0.0, 1.0, 0.0, Duration::from_millis(400));
            let y = a.sample(0.0, 1.0, 0.0, Duration::from_micros(400001));
            assert!(((y.value - x.value) / 0.000001 - x.velocity).abs() < 0.00001);
        }
    }
}

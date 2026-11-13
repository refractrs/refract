//! Tracked input preparation and host-owned drawing caches.

use super::*;
use crate::{AnyView, Description, Element, ElementKind, Handler, View, ViewContext, ViewError};
use std::{any::Any, fmt, rc::Rc, time::Duration};

/// Read-only graph access during drawing preparation. Reads belong to the
/// canvas mount, not the containing view. No runtime is available while painting.
pub struct DrawingInputs<'a, 'b> {
    cx: &'a mut ViewContext<'b>,
}

impl DrawingInputs<'_, '_> {
    pub fn read<T: 'static>(&self, value: &crate::Read<T>) -> T {
        self.cx.read(value)
    }

    pub fn with<T: 'static, R>(&self, value: &crate::Read<T>, f: impl FnOnce(&T) -> R) -> R {
        self.cx.with(value, f)
    }

    pub fn environment<K: crate::EnvironmentKey>(&mut self) -> K::Value {
        self.cx.environment::<K>()
    }
}

/// Fixed-dimensional presentation data. Components must be finite and keep
/// their order and count across updates. Non-component fields snap to target.
pub trait Animatable: Clone + PartialEq + 'static {
    fn components(&self) -> Vec<f64>;

    fn with_components(&self, components: &[f64]) -> Self;
}

impl Animatable for f64 {
    fn components(&self) -> Vec<f64> {
        vec![*self]
    }

    fn with_components(&self, values: &[f64]) -> Self {
        values[0]
    }
}

impl Animatable for f32 {
    fn components(&self) -> Vec<f64> {
        vec![f64::from(*self)]
    }

    fn with_components(&self, values: &[f64]) -> Self {
        values[0].clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32
    }
}

impl Animatable for Point {
    fn components(&self) -> Vec<f64> {
        vec![self.x, self.y]
    }

    fn with_components(&self, values: &[f64]) -> Self {
        Self::new(values[0], values[1])
    }
}

impl Animatable for Size {
    fn components(&self) -> Vec<f64> {
        vec![self.width, self.height]
    }

    fn with_components(&self, values: &[f64]) -> Self {
        Self::new(values[0], values[1])
    }
}

impl Animatable for crate::Color {
    fn components(&self) -> Vec<f64> {
        // Premultiplication keeps transparent RGB from tinting intermediate colors.
        let alpha = f64::from(self.3) / 255.0;
        vec![
            f64::from(self.0) / 255.0 * alpha,
            f64::from(self.1) / 255.0 * alpha,
            f64::from(self.2) / 255.0 * alpha,
            alpha,
        ]
    }

    fn with_components(&self, values: &[f64]) -> Self {
        let alpha = values[3].clamp(0.0, 1.0);
        if alpha <= 0.000001 {
            return Self(0, 0, 0, 0);
        }
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self(
            byte(values[0] / alpha),
            byte(values[1] / alpha),
            byte(values[2] / alpha),
            byte(alpha),
        )
    }
}

impl<A: Animatable, B: Animatable> Animatable for (A, B) {
    fn components(&self) -> Vec<f64> {
        let mut values = self.0.components();
        values.extend(self.1.components());
        values
    }

    fn with_components(&self, values: &[f64]) -> Self {
        let n = self.0.components().len();
        (
            self.0.with_components(&values[..n]),
            self.1.with_components(&values[n..]),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasSchedule {
    #[default]
    OnChange,
    /// Retain scheduled time without requesting frames.
    Paused,
    /// Pauses while hidden and while reduced motion is requested.
    Animation,
}

impl CanvasSchedule {
    pub fn animation() -> Self {
        Self::Animation
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasFrame {
    /// Actual content size, before the view-box transform.
    pub size: Size,
    /// Logical drawing bounds: the view box, or the content box at the origin.
    pub bounds: Rect,
    /// Visible animation time. Zero for unscheduled and reduced-motion drawings.
    pub elapsed: Duration,
}

type Draw<I> = dyn Fn(&mut GraphicsContext, &CanvasFrame, &I);
type Prepare<I> = dyn Fn(&mut DrawingInputs<'_, '_>) -> I;
type Components<I> = fn(&I) -> Vec<f64>;
type Replace<I> = fn(&I, &[f64]) -> I;

trait Prepared: Any {
    fn as_any(&self) -> &dyn Any;

    fn draw(&self, g: &mut GraphicsContext, frame: &CanvasFrame);

    fn equal(&self, other: &dyn Prepared) -> bool;

    fn components(&self) -> Option<Vec<f64>>;

    fn sampled(&self, values: &[f64]) -> Rc<dyn Prepared>;
}

struct PreparedInputs<I> {
    input: I,
    draw: Rc<Draw<I>>,
    animation: Option<(Components<I>, Replace<I>)>,
}

impl<I: Clone + PartialEq + 'static> Prepared for PreparedInputs<I> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn draw(&self, g: &mut GraphicsContext, frame: &CanvasFrame) {
        (self.draw)(g, frame, &self.input);
    }

    fn equal(&self, other: &dyn Prepared) -> bool {
        other.as_any().downcast_ref::<Self>().is_some_and(|p| {
            Rc::ptr_eq(&self.draw, &p.draw)
                && self.input == p.input
                && self.animation.is_some() == p.animation.is_some()
        })
    }

    fn components(&self) -> Option<Vec<f64>> {
        self.animation.map(|(get, _)| get(&self.input))
    }

    fn sampled(&self, values: &[f64]) -> Rc<dyn Prepared> {
        let input = self.animation.map_or_else(
            || self.input.clone(),
            |(_, replace)| replace(&self.input, values),
        );
        Rc::new(Self {
            input,
            draw: self.draw.clone(),
            animation: self.animation,
        })
    }
}

/// Committed drawing snapshot. Drawing receives no graph runtime or host API.
#[derive(Clone)]
pub struct CanvasContent {
    prepared: Rc<dyn Prepared>,
    pub view_box: Option<Rect>,
    pub schedule: CanvasSchedule,
}

impl fmt::Debug for CanvasContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasContent")
            .field("view_box", &self.view_box)
            .field("schedule", &self.schedule)
            .finish_non_exhaustive()
    }
}

impl PartialEq for CanvasContent {
    fn eq(&self, other: &Self) -> bool {
        self.view_box == other.view_box
            && self.schedule == other.schedule
            && (Rc::ptr_eq(&self.prepared, &other.prepared)
                || self.prepared.equal(&*other.prepared))
    }
}

impl CanvasContent {
    pub(crate) fn components(&self) -> Option<Vec<f64>> {
        self.prepared.components()
    }

    pub(crate) fn compatible(&self, other: &Self) -> bool {
        self.prepared.as_any().type_id() == other.prepared.as_any().type_id()
    }

    pub(crate) fn sampled(&self, values: &[f64]) -> Self {
        Self {
            prepared: self.prepared.sampled(values),
            ..self.clone()
        }
    }

    pub fn validate(&self) -> Result<(), ViewError> {
        if self
            .view_box
            .is_some_and(|r| !r.valid() || r.width == 0.0 || r.height == 0.0)
            || self
                .components()
                .is_some_and(|v| v.iter().any(|v| !v.is_finite()))
        {
            return Err(ViewError::new(
                "canvas bounds and animation components must be finite",
            ));
        }
        Ok(())
    }

    pub fn transform(&self, size: Size) -> crate::Affine2d {
        let Some(b) = self.view_box else {
            return crate::Affine2d::IDENTITY;
        };
        let scale = (size.width / b.width).min(size.height / b.height).max(0.0);
        crate::Affine2d([
            scale,
            0.0,
            0.0,
            scale,
            (size.width - b.width * scale) / 2.0 - b.x * scale,
            (size.height - b.height * scale) / 2.0 - b.y * scale,
        ])
    }

    pub fn draw(&self, size: Size, elapsed: Duration) -> Result<Rc<[DrawingCommand]>, ViewError> {
        self.validate()?;
        if !Rect::new(0.0, 0.0, size.width, size.height).valid() {
            return Err(ViewError::new("invalid canvas size"));
        }
        let frame = CanvasFrame {
            size,
            bounds: self
                .view_box
                .unwrap_or(Rect::new(0.0, 0.0, size.width, size.height)),
            elapsed,
        };
        let mut g = GraphicsContext::new();
        g.transformed(self.transform(size), |g| self.prepared.draw(g, &frame));
        g.finish()
    }
}

/// A single layout/event target with an independently tracked input preparation.
pub struct Canvas<I> {
    prepare: Rc<Prepare<I>>,
    draw: Rc<Draw<I>>,
    animation: Option<(Components<I>, Replace<I>)>,
    element: Element,
    handler: Option<Handler>,
    view_box: Option<Rect>,
    schedule: CanvasSchedule,
}

pub fn canvas(draw: impl Fn(&mut GraphicsContext, &CanvasFrame) + 'static) -> Canvas<()> {
    canvas_with(|_| (), move |g, frame, _| draw(g, frame))
}

/// Prepare graph-dependent inputs at the canvas mount. Drawing uses the committed
/// snapshot and may run each frame without reevaluating the view.
pub fn canvas_with<I: Clone + PartialEq + 'static>(
    prepare: impl Fn(&mut DrawingInputs<'_, '_>) -> I + 'static,
    draw: impl Fn(&mut GraphicsContext, &CanvasFrame, &I) + 'static,
) -> Canvas<I> {
    Canvas {
        prepare: Rc::new(prepare),
        draw: Rc::new(draw),
        animation: None,
        element: Element::new(ElementKind::Canvas),
        handler: None,
        view_box: None,
        schedule: CanvasSchedule::OnChange,
    }
}

impl<I> crate::modifiers::ElementBuilder for Canvas<I> {
    fn element_mut(&mut self) -> &mut Element {
        &mut self.element
    }

    fn handler_mut(&mut self) -> &mut Option<Handler> {
        &mut self.handler
    }
}

impl<I> Canvas<I> {
    crate::modifiers::element_modifiers!(pub Self, crate::modifiers::configure);
    crate::modifiers::event_modifiers!(pub Self, crate::modifiers::configure_handler);

    pub fn view_box(mut self, bounds: Rect) -> Self {
        self.view_box = Some(bounds);
        self
    }

    pub fn schedule(mut self, schedule: CanvasSchedule) -> Self {
        self.schedule = schedule;
        self
    }

    pub fn animatable_inputs(mut self) -> Self
    where
        I: Animatable,
    {
        self.animation = Some((I::components, I::with_components));
        self
    }
}

impl<I: Clone + PartialEq + 'static> View for Canvas<I> {
    fn body(&self, cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        let input = (self.prepare)(&mut DrawingInputs { cx });
        let mut element = self.element.clone();
        element.canvas = Some(CanvasContent {
            prepared: Rc::new(PreparedInputs {
                input,
                draw: self.draw.clone(),
                animation: self.animation,
            }),
            view_box: self.view_box,
            schedule: self.schedule,
        });
        Ok(crate::view::Described(Description::Element(
            Box::new(element),
            Vec::<AnyView>::new(),
            self.handler.clone(),
        )))
    }
}

/// Host-owned cache. Painting never evaluates views or reads the graph.
#[derive(Default)]
pub struct CanvasCache {
    content: Option<CanvasContent>,
    size: Size,
    elapsed: Duration,
    last_tick: Option<Duration>,
    rendered_elapsed: Duration,
    commands: Option<Rc<[DrawingCommand]>>,
}

impl CanvasCache {
    pub fn pause(&mut self) {
        // Preserve the phase, but exclude the paused interval from the next tick.
        self.last_tick = None;
    }

    pub fn render(
        &mut self,
        content: &CanvasContent,
        size: Size,
        now: Duration,
        visible: bool,
        reduced_motion: bool,
    ) -> Result<Rc<[DrawingCommand]>, ViewError> {
        let scheduled = content.schedule == CanvasSchedule::Animation && !reduced_motion;
        if scheduled && visible {
            if let Some(last) = self.last_tick {
                self.elapsed += now.saturating_sub(last);
            }
            self.last_tick = Some(now);
        } else {
            self.pause();
        }
        let elapsed = if reduced_motion || content.schedule == CanvasSchedule::OnChange {
            Duration::ZERO
        } else {
            self.elapsed
        };
        if self.content.as_ref() != Some(content)
            || self.size != size
            || self.rendered_elapsed != elapsed
            || self.commands.is_none()
        {
            let commands = content.draw(size, elapsed)?;
            self.commands = Some(commands);
            self.content = Some(content.clone());
            self.size = size;
            self.rendered_elapsed = elapsed;
        }
        Ok(self.commands.as_ref().unwrap().clone())
    }
}

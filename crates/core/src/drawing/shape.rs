//! Shapes generate geometry for the bounds supplied by layout.
use super::*;
use crate::{Color, View, ViewContext, ViewError};

pub trait Shape: 'static {
    fn path(&self, bounds: Rect) -> Path;
}
impl Shape for Path {
    fn path(&self, _bounds: Rect) -> Path {
        self.clone()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Rectangle;
impl Shape for Rectangle {
    fn path(&self, bounds: Rect) -> Path {
        Path::rectangle(bounds)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Ellipse;
impl Shape for Ellipse {
    fn path(&self, bounds: Rect) -> Path {
        Path::ellipse(bounds)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Circle;
impl Shape for Circle {
    fn path(&self, bounds: Rect) -> Path {
        let diameter = bounds.width.min(bounds.height);
        let center = bounds.center();
        Path::ellipse(Rect::new(
            center.x - diameter / 2.0,
            center.y - diameter / 2.0,
            diameter,
            diameter,
        ))
    }
}

pub struct ShapeView<S> {
    shape: std::rc::Rc<S>,
    fill: Option<Paint>,
    stroke: Option<(Paint, StrokeStyle)>,
}
pub fn shape<S: Shape>(shape: S) -> ShapeView<S> {
    ShapeView {
        shape: std::rc::Rc::new(shape),
        fill: Some(Color::BLACK.into()),
        stroke: None,
    }
}
impl<S: Shape> ShapeView<S> {
    pub fn fill(mut self, paint: impl Into<Paint>) -> Self {
        self.fill = Some(paint.into());
        self
    }
    pub fn no_fill(mut self) -> Self {
        self.fill = None;
        self
    }
    pub fn stroke(mut self, paint: impl Into<Paint>, style: impl Into<StrokeStyle>) -> Self {
        self.stroke = Some((paint.into(), style.into()));
        self
    }
}
impl<S: Shape> View for ShapeView<S> {
    fn body(&self, _cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        let shape = self.shape.clone();
        let fill = self.fill.clone();
        let stroke = self.stroke.clone();
        Ok(canvas(move |g, frame| {
            let path = shape.path(frame.bounds);
            if let Some(paint) = &fill {
                g.fill(&path, paint);
            }
            if let Some((paint, style)) = &stroke {
                g.stroke(&path, paint, style);
            }
        }))
    }
}

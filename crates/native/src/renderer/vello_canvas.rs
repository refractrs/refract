//! Encode portable drawing trees, including bounded local-space effects.

use super::vello_scene::{color, Encoder};
use refract_core::{
    drawing_bounds, DrawingCommand, FillRule, Layer, LineCap, LineJoin, Paint, Path, PathElement,
    StrokeStyle,
};
use vello::{
    kurbo::{Affine, BezPath, Cap, Join, Rect, Stroke},
    peniko::{BlendMode, Brush, Compose, Fill, Gradient, Mix},
    Scene,
};

fn path(path: &Path) -> BezPath {
    let mut result = BezPath::new();
    for el in path.elements() {
        let p = |p: refract_core::Point| (p.x, p.y);
        match *el {
            PathElement::MoveTo(a) => result.move_to(p(a)),
            PathElement::LineTo(a) => result.line_to(p(a)),
            PathElement::QuadTo(a, b) => result.quad_to(p(a), p(b)),
            PathElement::CubicTo(a, b, c) => result.curve_to(p(a), p(b), p(c)),
            PathElement::Close => result.close_path(),
        }
    }
    result
}

fn paint(paint: &Paint) -> Brush {
    let gradient = match paint {
        Paint::Solid(c) => return color(*c).into(),
        Paint::Linear { start, end, stops } => {
            Gradient::new_linear((start.x, start.y), (end.x, end.y)).with_stops(
                stops
                    .iter()
                    .map(|s| vello::peniko::ColorStop {
                        offset: s.offset,
                        color: color(s.color).into(),
                    })
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
        }
        Paint::Radial {
            center,
            radius,
            stops,
        } => Gradient::new_radial((center.x, center.y), *radius as f32).with_stops(
            stops
                .iter()
                .map(|s| vello::peniko::ColorStop {
                    offset: s.offset,
                    color: color(s.color).into(),
                })
                .collect::<Vec<_>>()
                .as_slice(),
        ),
    };
    gradient.into()
}

fn stroke(style: &StrokeStyle) -> Stroke {
    Stroke::new(style.width)
        .with_caps(match style.line_cap {
            LineCap::Butt => Cap::Butt,
            LineCap::Round => Cap::Round,
            LineCap::Square => Cap::Square,
        })
        .with_join(match style.line_join {
            LineJoin::Miter => Join::Miter,
            LineJoin::Round => Join::Round,
            LineJoin::Bevel => Join::Bevel,
        })
        .with_miter_limit(style.miter_limit)
        .with_dashes(style.dash_offset, style.dash.iter().copied())
}

fn fill(rule: FillRule) -> Fill {
    match rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    }
}

fn rect(r: refract_core::Rect) -> Rect {
    Rect::new(r.x, r.y, r.x + r.width, r.y + r.height)
}

impl Encoder<'_> {
    pub(super) fn canvas(
        &mut self,
        scene: &mut Scene,
        commands: &[DrawingCommand],
        bounds: super::Rect,
        transform: Affine,
    ) -> crate::Result<()> {
        let clip = Rect::new(
            bounds.x.into(),
            bounds.y.into(),
            (bounds.x + bounds.width).into(),
            (bounds.y + bounds.height).into(),
        );
        let device = Affine::scale(self.scale_factor) * transform;
        scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, device, &clip);
        self.drawing(
            scene,
            commands,
            device * Affine::translate((bounds.x.into(), bounds.y.into())),
        )?;
        scene.pop_layer();
        Ok(())
    }

    fn drawing(
        &mut self,
        scene: &mut Scene,
        commands: &[DrawingCommand],
        transform: Affine,
    ) -> crate::Result<()> {
        for command in commands {
            match command {
                DrawingCommand::Fill {
                    path: p,
                    paint: b,
                    rule,
                } => scene.fill(fill(*rule), transform, &paint(b), None, &path(p)),
                DrawingCommand::Stroke {
                    path: p,
                    paint: b,
                    style,
                } => scene.stroke(&stroke(style), transform, &paint(b), None, &path(p)),
                DrawingCommand::Group {
                    transform: local,
                    clip,
                    layer,
                    commands,
                } => {
                    let transform = transform * Affine::new(local.0);
                    if let Some((p, rule)) = clip {
                        scene.push_layer(
                            fill(*rule),
                            BlendMode::default(),
                            1.0,
                            transform,
                            &path(p),
                        );
                    }
                    if let Some(layer) = layer {
                        self.drawing_layer(scene, commands, transform, layer)?;
                    } else {
                        self.drawing(scene, commands, transform)?;
                    }
                    if clip.is_some() {
                        scene.pop_layer();
                    }
                }
            }
        }
        Ok(())
    }

    fn drawing_layer(
        &mut self,
        scene: &mut Scene,
        commands: &[DrawingCommand],
        transform: Affine,
        layer: &Layer,
    ) -> crate::Result<()> {
        let Some(bounds) = drawing_bounds(commands) else {
            return Ok(());
        };
        // Filter in the group's local coordinates before transforming the result.
        // This preserves anisotropic scaling and rotated Gaussian coverage.
        let local = Affine::scale(self.scale_factor);
        let area = local.transform_rect_bbox(rect(bounds.inflate(layer.blur * 3.0 + 1.0)));
        if area.width() <= 0.0 || area.height() <= 0.0 {
            return Ok(());
        }
        let mut content = Scene::new();
        self.drawing(&mut content, commands, local)?;
        if layer.blur > 0.0 {
            content = self.effects.blur(
                self.renderer,
                self.device,
                self.queue,
                &content,
                area,
                layer.blur * self.scale_factor,
                false,
            )?;
        }

        // Destination-in retains the blurred content in proportion to mask alpha.
        if let Some(mask) = &layer.mask {
            content.push_layer(
                Fill::NonZero,
                BlendMode::new(Mix::Normal, Compose::DestIn),
                1.0,
                Affine::IDENTITY,
                &area,
            );
            content.fill(
                Fill::NonZero,
                local,
                &paint(&mask.paint),
                None,
                &path(&mask.path),
            );
            content.pop_layer();
        }
        let blend = match layer.blend_mode {
            refract_core::BlendMode::Normal => BlendMode::default(),
            refract_core::BlendMode::Multiply => BlendMode::new(Mix::Multiply, Compose::SrcOver),
            refract_core::BlendMode::Screen => BlendMode::new(Mix::Screen, Compose::SrcOver),
            refract_core::BlendMode::Plus => BlendMode::new(Mix::Normal, Compose::PlusLighter),
        };
        let composite = transform * local.inverse();
        scene.push_layer(Fill::NonZero, blend, layer.opacity, composite, &area);
        scene.append(&content, Some(composite));
        scene.pop_layer();
        Ok(())
    }
}

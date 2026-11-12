//! Scene encoding and nested effect/backdrop boundaries.
use super::vello_effects::Effects;
use super::vello_geometry::{rounded, RoundedBox};
use super::{DrawCommand, ImageId, RasterImage};
use crate::Result;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use vello::{
    kurbo::{Affine, Rect},
    peniko::{BlendMode, Blob, Color, Fill, ImageAlphaType, ImageData, ImageFormat},
    wgpu, Scene,
};

pub(super) fn color(c: refract_core::Color) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, c.3)
}

#[derive(Default)]
pub(super) struct SceneResources {
    images: HashMap<ImageId, (Weak<RasterImage>, ImageData)>,
    shadows: super::vello_shadow::ShadowCache,
}

impl SceneResources {
    pub fn begin_frame(&mut self) {
        self.images
            .retain(|_, (resource, _)| resource.strong_count() != 0);
    }
}

pub(super) struct Encoder<'a> {
    pub resources: &'a mut SceneResources,
    pub effects: &'a mut Effects,
    pub renderer: &'a mut vello::Renderer,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub scale_factor: f64,
    pub has_backdrop: bool,
}

// The prefix contains earlier paint and open ancestor clips. Close those clips
// only when taking a snapshot, so a backdrop never samples future siblings.
#[derive(Clone, Default)]
pub(super) struct Backdrop {
    pub scene: Scene,
    pub clips: usize,
}
impl Backdrop {
    fn snapshot(&self, scene: &Scene) -> Scene {
        let mut result = self.scene.clone();
        result.append(scene, None);
        for _ in 0..self.clips {
            result.pop_layer();
        }
        result
    }
}

impl Encoder<'_> {
    pub fn encode(
        &mut self,
        commands: &mut std::slice::Iter<'_, DrawCommand>,
        scene: &mut Scene,
        backdrop: &Backdrop,
        extent: Rect,
    ) -> Result<()> {
        self.encode_at(commands, scene, backdrop, extent, Affine::IDENTITY)
    }

    fn encode_at(&mut self, commands: &mut std::slice::Iter<'_, DrawCommand>, scene: &mut Scene,
        backdrop: &Backdrop, extent: Rect, transform: Affine) -> Result<()> {
        let scale = Affine::scale(self.scale_factor) * transform;
        while let Some(command) = commands.next() {
            match command {
                DrawCommand::PopLayer => return Ok(()),
                DrawCommand::PushTransform(matrix) => {
                    self.encode_at(commands, scene, backdrop, extent, transform * Affine::new(matrix.0))?;
                }
                DrawCommand::PushOpacity(opacity) => {
                    let mut content = Scene::new();
                    self.encode_at(commands, &mut content, &Backdrop::default(), extent, transform)?;
                    scene.push_layer(Fill::NonZero, BlendMode::default(), *opacity, Affine::IDENTITY, &extent);
                    scene.append(&content, None);
                    scene.pop_layer();
                }
                DrawCommand::PushEffects {
                    rect,
                    radius,
                    blur,
                    backdrop_blur,
                    opacity,
                } => {
                    let sigma = f64::from(*blur) * self.scale_factor;
                    let input_extent = extent.inflate(3.0 * sigma, 3.0 * sigma);
                    let mut content = Scene::new();
                    if *backdrop_blur > 0.0 && rect.width > 0.0 && rect.height > 0.0 {
                        let source = backdrop.snapshot(scene);
                        let shape = rounded(*rect, *radius);
                        let area = scale.transform_rect_bbox(Rect::new(
                            f64::from(rect.x),
                            f64::from(rect.y),
                            f64::from(rect.x) + f64::from(rect.width),
                            f64::from(rect.y) + f64::from(rect.height),
                        ));
                        if area.intersect(input_extent).area() > 0.0 {
                            let amount = f64::from(*backdrop_blur) * self.scale_factor;
                            let filtered = self.effects.blur(
                                self.renderer,
                                self.device,
                                self.queue,
                                &source,
                                area,
                                amount,
                                true,
                            )?;
                            content.push_layer(
                                Fill::NonZero,
                                BlendMode::default(),
                                1.0,
                                scale,
                                &shape,
                            );
                            content.append(&filtered, None);
                            content.pop_layer();
                        }
                    }
                    // Filters and backdrop filters establish a new backdrop root.
                    self.encode_at(commands, &mut content, &Backdrop::default(), input_extent, transform)?;
                    if sigma > 0.0 {
                        content = self.effects.blur(
                            self.renderer,
                            self.device,
                            self.queue,
                            &content,
                            input_extent,
                            sigma,
                            false,
                        )?;
                    }
                    scene.push_layer(
                        Fill::NonZero,
                        BlendMode::default(),
                        *opacity,
                        Affine::IDENTITY,
                        &extent,
                    );
                    scene.append(&content, None);
                    scene.pop_layer();
                }
                DrawCommand::PushLayer { .. } | DrawCommand::PushInsetLayer { .. } => {
                    let opacity = match command {
                        DrawCommand::PushLayer { opacity, .. } => *opacity,
                        _ => 1.0,
                    };
                    let mut prefix = if self.has_backdrop && opacity == 1.0 {
                        let mut prefix = backdrop.clone();
                        prefix.scene.append(scene, None);
                        prefix
                    } else {
                        Backdrop::default()
                    };
                    if self.has_backdrop {
                        push_clip(&mut prefix.scene, command, scale, 1.0);
                        prefix.clips += 1;
                    }
                    let mut content = Scene::new();
                    self.encode_at(commands, &mut content, &prefix, extent, transform)?;
                    push_clip(scene, command, scale, opacity);
                    scene.append(&content, None);
                    scene.pop_layer();
                }
                _ => {
                    if transform == Affine::IDENTITY { self.primitive(scene, command); }
                    else {
                        let mut primitive = Scene::new();
                        self.primitive(&mut primitive, command);
                        scene.append(&primitive, Some(Affine::scale(self.scale_factor) * transform * Affine::scale(1.0 / self.scale_factor)));
                    }
                }
            }
        }
        Ok(())
    }

    fn primitive(&mut self, scene: &mut Scene, command: &DrawCommand) {
        let scale_factor = self.scale_factor;
        let scale = Affine::scale(scale_factor);
        match command {
            DrawCommand::BoxShadow {
                rect,
                radius,
                shadow,
            } => {
                super::vello_shadow::draw(
                    scene,
                    &mut self.resources.shadows,
                    scale_factor,
                    *rect,
                    *radius,
                    *shadow,
                );
            }
            DrawCommand::Image { rect, image } => {
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    return;
                }
                let (_, data) = self.resources.images.entry(image.id()).or_insert_with(|| {
                    (
                        Arc::downgrade(image),
                        ImageData {
                            // One shared pixel allocation; Blob identity stays stable
                            // so Vello can reuse its image atlas across frames.
                            data: Blob::new(Arc::new(image.rgba().clone())),
                            format: ImageFormat::Rgba8,
                            alpha_type: ImageAlphaType::Alpha,
                            width: image.width(),
                            height: image.height(),
                        },
                    )
                });
                let transform = scale
                    * Affine::translate((rect.x as f64, rect.y as f64))
                    * Affine::scale_non_uniform(
                        rect.width as f64 / image.width() as f64,
                        rect.height as f64 / image.height() as f64,
                    );
                scene.draw_image(&*data, transform);
            }
            DrawCommand::Fill {
                rect,
                radius,
                color: brush,
            } => {
                scene.fill(
                    Fill::NonZero,
                    scale,
                    color(*brush),
                    None,
                    &rounded(*rect, *radius),
                );
            }
            DrawCommand::LinearGradient {
                rect,
                radius,
                gradient,
            } => {
                let (start, end) = gradient.endpoints(rect.width as f64, rect.height as f64);
                let stops: Vec<_> = gradient
                    .stops
                    .iter()
                    .map(|stop| vello::peniko::ColorStop {
                        offset: stop.offset,
                        color: color(stop.color).into(),
                    })
                    .collect();
                let brush = vello::peniko::Gradient::new_linear(
                    (rect.x as f64 + start.0, rect.y as f64 + start.1),
                    (rect.x as f64 + end.0, rect.y as f64 + end.1),
                )
                .with_stops(stops.as_slice());
                scene.fill(Fill::NonZero, scale, &brush, None, &rounded(*rect, *radius));
            }
            DrawCommand::RadialGradient {
                rect,
                radius,
                gradient,
            } => {
                let stops: Vec<_> = gradient
                    .stops
                    .iter()
                    .map(|stop| vello::peniko::ColorStop {
                        offset: stop.offset,
                        color: color(stop.color).into(),
                    })
                    .collect();
                let brush = vello::peniko::Gradient::new_radial(
                    (
                        f64::from(rect.x) + f64::from(rect.width) * f64::from(gradient.center.0),
                        f64::from(rect.y) + f64::from(rect.height) * f64::from(gradient.center.1),
                    ),
                    gradient.radius,
                )
                .with_stops(stops.as_slice());
                scene.fill(Fill::NonZero, scale, &brush, None, &rounded(*rect, *radius));
            }
            DrawCommand::Stroke {
                rect,
                radius,
                width,
                color: brush,
            } => {
                super::vello_geometry::border(
                    scene,
                    scale,
                    *rect,
                    *radius,
                    refract_core::Border::all(*width, *brush),
                );
            }
            DrawCommand::Border {
                rect,
                radius,
                border,
            } => {
                super::vello_geometry::border(scene, scale, *rect, *radius, *border);
            }
            DrawCommand::Glyphs {
                origin,
                color: brush,
                run,
            } => {
                scene
                    .draw_glyphs(&run.font)
                    .font_size(run.font_size)
                    .normalized_coords(&run.normalized_coords)
                    .glyph_transform(
                        run.skew
                            .map(|angle| Affine::skew(f64::from(angle).to_radians().tan(), 0.0)),
                    )
                    .transform(scale * Affine::translate((origin.x as f64, origin.y as f64)))
                    .brush(color(*brush))
                    .draw(
                        Fill::NonZero,
                        run.glyphs.iter().map(|g| vello::Glyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        }),
                    );
            }
            _ => unreachable!("layer commands are handled by encode"),
        }
    }
}

fn push_clip(scene: &mut Scene, command: &DrawCommand, scale: Affine, opacity: f32) {
    match command {
        DrawCommand::PushLayer { clip, radius, .. } => scene.push_layer(
            Fill::NonZero,
            BlendMode::default(),
            opacity,
            scale,
            &rounded(*clip, *radius),
        ),
        DrawCommand::PushInsetLayer {
            rect,
            radius,
            insets,
        } => scene.push_layer(
            Fill::NonZero,
            BlendMode::default(),
            1.0,
            scale,
            &RoundedBox::new(*rect, *radius).inset(*insets).path(),
        ),
        _ => unreachable!(),
    }
}

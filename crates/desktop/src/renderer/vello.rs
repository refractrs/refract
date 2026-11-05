//! Vello implementation of the drawing contract. GPU objects stay in this module.

use super::vello_geometry::{RoundedBox, rounded};
use super::{DrawCommand, Frame, ImageId, Presentation, RasterImage, Renderer};
use crate::{DesktopError, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use vello::{
    AaConfig, AaSupport, RenderParams, RendererOptions, Scene,
    kurbo::Affine,
    peniko::{BlendMode, Blob, Color, Fill, ImageAlphaType, ImageData, ImageFormat},
    util::{RenderContext, RenderSurface},
    wgpu,
};
use winit::window::Window;

/// GPU scene renderer with an owned presentation surface and retained Vello caches.
pub struct VelloRenderer {
    // Drop the surface before its window/device owners.
    surface: RenderSurface<'static>,
    renderer: vello::Renderer,
    context: RenderContext,
    window: Arc<Window>,
    scene: Scene,
    images: HashMap<ImageId, (Weak<RasterImage>, ImageData)>,
    shadows: super::vello_shadow::ShadowCache,
}

impl VelloRenderer {
    /// Initialize GPU resources for a live Winit window on the UI thread.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let mut context = RenderContext::new();
        let size = window.inner_size();
        let surface = pollster::block_on(context.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .map_err(render_error)?;
        configure_window_presentation(&surface.surface, &window);
        let renderer = vello::Renderer::new(
            &context.devices[surface.dev_id].device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                ..RendererOptions::default()
            },
        )
        .map_err(render_error)?;
        Ok(Self {
            surface,
            renderer,
            context,
            window,
            scene: Scene::new(),
            images: HashMap::new(),
            shadows: super::vello_shadow::ShadowCache::default(),
        })
    }

    fn encode(&mut self, frame: &Frame) {
        self.scene.reset();
        self.images
            .retain(|_, (resource, _)| resource.strong_count() != 0);
        let scale = Affine::scale(frame.scale_factor);
        for command in &frame.commands {
            match command {
                DrawCommand::BoxShadow {
                    rect,
                    radius,
                    shadow,
                } => {
                    super::vello_shadow::draw(
                        &mut self.scene,
                        &mut self.shadows,
                        frame.scale_factor,
                        *rect,
                        *radius,
                        *shadow,
                    );
                }
                DrawCommand::Image { rect, image } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    let (_, data) = self.images.entry(image.id()).or_insert_with(|| {
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
                    self.scene.draw_image(&*data, transform);
                }
                DrawCommand::Fill {
                    rect,
                    radius,
                    color: brush,
                } => {
                    self.scene.fill(
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
                    self.scene
                        .fill(Fill::NonZero, scale, &brush, None, &rounded(*rect, *radius));
                }
                DrawCommand::Stroke {
                    rect,
                    radius,
                    width,
                    color: brush,
                } => {
                    super::vello_geometry::border(
                        &mut self.scene,
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
                    super::vello_geometry::border(&mut self.scene, scale, *rect, *radius, *border);
                }
                DrawCommand::PushInsetLayer {
                    rect,
                    radius,
                    insets,
                } => {
                    self.scene.push_clip_layer(
                        Fill::NonZero,
                        scale,
                        &RoundedBox::new(*rect, *radius).inset(*insets).path(),
                    );
                }
                DrawCommand::Glyphs {
                    origin,
                    color: brush,
                    run,
                } => {
                    self.scene
                        .draw_glyphs(&run.font)
                        .font_size(run.font_size)
                        .normalized_coords(&run.normalized_coords)
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
                DrawCommand::PushLayer {
                    clip,
                    radius,
                    opacity,
                } => {
                    self.scene.push_layer(
                        Fill::NonZero,
                        BlendMode::default(),
                        *opacity,
                        scale,
                        &rounded(*clip, *radius),
                    );
                }
                DrawCommand::PopLayer => self.scene.pop_layer(),
            }
        }
    }
}

impl Renderer for VelloRenderer {
    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width > 0
            && height > 0
            && (self.surface.config.width != width || self.surface.config.height != height)
        {
            self.context
                .resize_surface(&mut self.surface, width, height);
        }
        Ok(())
    }

    fn render(&mut self, frame: &Frame) -> Result<Presentation> {
        // Live edge resizing and display/window-manager transitions need
        // different fallback behavior for the previous drawable.
        configure_window_presentation(&self.surface.surface, &self.window);
        self.encode(frame);
        let (texture, reconfigure) = match self.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            // Suboptimal drawables are still usable. Present this frame instead
            // of freezing live resize behind the transient-error retry delay.
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(Presentation::Retry);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.context.configure_surface(&self.surface);
                return Ok(Presentation::Retry);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = pollster::block_on(self.context.create_surface(
                    self.window.clone(),
                    self.surface.config.width,
                    self.surface.config.height,
                    wgpu::PresentMode::AutoVsync,
                ))
                .map_err(render_error)?;
                configure_window_presentation(&self.surface.surface, &self.window);
                self.renderer = vello::Renderer::new(
                    &self.context.devices[self.surface.dev_id].device,
                    RendererOptions {
                        antialiasing_support: AaSupport::area_only(),
                        ..RendererOptions::default()
                    },
                )
                .map_err(render_error)?;
                return Ok(Presentation::Retry);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(DesktopError::Render("surface validation failed".into()));
            }
        };
        let device = &self.context.devices[self.surface.dev_id];
        self.renderer
            .render_to_texture(
                &device.device,
                &device.queue,
                &self.scene,
                &self.surface.target_view,
                &RenderParams {
                    base_color: color(frame.background),
                    width: self.surface.config.width,
                    height: self.surface.config.height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(render_error)?;
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Refract present"),
            });
        self.surface.blitter.copy(
            &device.device,
            &mut encoder,
            &self.surface.target_view,
            &view,
        );
        device.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        device.queue.present(texture);
        if reconfigure {
            self.context.configure_surface(&self.surface);
        }
        Ok(Presentation::Presented)
    }
}

fn color(c: refract_core::Color) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, c.3)
}
fn render_error(error: impl std::fmt::Display) -> DesktopError {
    DesktopError::Render(error.to_string())
}

/// Anchor old pixels only during AppKit live resizing to avoid apparent panel
/// movement. Outside that gesture, fill the layer: monitor/DPI changes and snap
/// animations can change its bounds or scale before the next drawable arrives.
/// Permanently anchoring leaves uncovered (black) regions during those changes.
fn configure_window_presentation(surface: &wgpu::Surface<'_>, window: &Window) {
    #[cfg(target_os = "macos")]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let live_resize = window.window_handle().ok().is_some_and(|handle| {
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return false;
            };
            // SAFETY: Winit's borrowed handle is a live NSView owned by window.
            // This renderer is used exclusively on the Winit/AppKit UI thread;
            // the reference is neither retained nor used beyond this call.
            let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
            view.inLiveResize()
        });
        // SAFETY: we only borrow the live Metal surface under wgpu's guard and
        // lock its layer. Nothing is destroyed or retained beyond the guard.
        // Renderer setup and surface recovery run on the Winit UI thread.
        if let Some(metal) = unsafe { surface.as_hal::<wgpu::hal::api::Metal>() } {
            let layer = metal.render_layer().lock();
            // wgpu honors this property by waiting for command scheduling
            // before presenting inside the Core Animation transaction.
            layer.setPresentsWithTransaction(true);
            // SAFETY: QuartzCore exports these immutable gravity constants.
            // Match the layer's coordinate orientation when anchoring pixels.
            let gravity = unsafe {
                if !live_resize {
                    objc2_quartz_core::kCAGravityResize
                } else if layer.contentsAreFlipped() {
                    objc2_quartz_core::kCAGravityBottomLeft
                } else {
                    objc2_quartz_core::kCAGravityTopLeft
                }
            };
            if &*layer.contentsGravity() != gravity {
                layer.setContentsGravity(gravity);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (surface, window);
}

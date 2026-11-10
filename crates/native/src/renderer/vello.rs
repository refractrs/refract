//! Vello implementation of the drawing contract. GPU objects stay in this module.

use super::vello_scene::{color, Backdrop, Encoder, SceneResources};
use super::{Frame, Presentation, Renderer};
use crate::{NativeError, Result};
use std::sync::Arc;
use vello::{
    kurbo::{Affine, Rect},
    peniko::Fill,
    util::{RenderContext, RenderSurface},
    wgpu, AaConfig, AaSupport, RenderParams, RendererOptions, Scene,
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
    resources: SceneResources,
    effects: super::vello_effects::Effects,
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
            resources: SceneResources::default(),
            effects: super::vello_effects::Effects::default(),
        })
    }

    fn encode(&mut self, frame: &Frame) -> Result<()> {
        self.effects.begin_frame(&mut self.renderer);
        self.resources.begin_frame();
        self.scene.reset();
        let extent = Rect::new(
            0.0,
            0.0,
            f64::from(self.surface.config.width),
            f64::from(self.surface.config.height),
        );
        let mut backdrop = Backdrop::default();
        backdrop.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(frame.background),
            None,
            &extent,
        );
        let device = &self.context.devices[self.surface.dev_id];
        Encoder {
            resources: &mut self.resources,
            effects: &mut self.effects,
            renderer: &mut self.renderer,
            device: &device.device,
            queue: &device.queue,
            scale_factor: frame.scale_factor,
            has_backdrop: frame.commands.iter().any(|c| matches!(c, super::DrawCommand::PushEffects { backdrop_blur, .. } if *backdrop_blur > 0.0)),
        }.encode(&mut frame.commands.iter(), &mut self.scene, &backdrop, extent)
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
        self.encode(frame)?;
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
                self.effects = super::vello_effects::Effects::default();
                return Ok(Presentation::Retry);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(NativeError::Render("surface validation failed".into()));
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

fn render_error(error: impl std::fmt::Display) -> NativeError {
    NativeError::Render(error.to_string())
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

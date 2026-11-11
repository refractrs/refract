//! Vello implementation of the drawing contract. GPU objects stay in this module.

#[cfg(feature = "debug-damage")]
use super::vello_debug::DamageOverlay;
use super::vello_painter::{Painter, RenderMode, RenderStats};
use super::{Frame, Presentation, Renderer};
use crate::{NativeError, Result};
use std::sync::Arc;
#[cfg(feature = "debug-damage")]
use std::time::Instant;
use vello::{
    util::{RenderContext, RenderSurface},
    wgpu,
};
use winit::window::Window;

/// GPU scene renderer with an owned presentation surface and retained Vello caches.
pub struct VelloRenderer {
    // Drop the surface before its window/device owners.
    surface: RenderSurface<'static>,
    painter: Painter,
    stats: RenderStats,
    #[cfg(feature = "debug-damage")]
    overlay: DamageOverlay,
    context: RenderContext,
    window: Arc<Window>,
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
        let painter = Painter::new(&context.devices[surface.dev_id].device)?;
        Ok(Self {
            surface,
            painter,
            stats: RenderStats::default(),
            #[cfg(feature = "debug-damage")]
            overlay: DamageOverlay::default(),
            context,
            window,
        })
    }

    /// Work submitted for the most recent successfully rendered frame.
    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    /// Select full rendering for diagnostics or comparisons. Incremental is the default.
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.painter.mode = mode;
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
        #[cfg(feature = "debug-damage")]
        {
            self.present_frame(Some(frame))
        }
        #[cfg(not(feature = "debug-damage"))]
        {
            self.present_frame(frame)
        }
    }

    #[cfg(feature = "debug-damage")]
    fn redraw_deadline(&self) -> Option<Instant> {
        self.overlay.deadline()
    }

    #[cfg(feature = "debug-damage")]
    fn refresh(&mut self) -> Result<Option<Presentation>> {
        self.present_frame(None).map(Some)
    }
}

impl VelloRenderer {
    fn paint_frame(&mut self, frame: &Frame) -> Result<()> {
        let device = &self.context.devices[self.surface.dev_id];
        self.stats = self.painter.paint(
            &device.device,
            &device.queue,
            frame,
            self.surface.config.width,
            self.surface.config.height,
        )?;
        Ok(())
    }

    fn present_frame(
        &mut self,
        #[cfg(feature = "debug-damage")] frame: Option<&Frame>,
        #[cfg(not(feature = "debug-damage"))] frame: &Frame,
    ) -> Result<Presentation> {
        // Live edge resizing and display/window-manager transitions need
        // different fallback behavior for the previous drawable.
        configure_window_presentation(&self.surface.surface, &self.window);
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
                let mode = self.painter.mode;
                self.painter = Painter::new(&self.context.devices[self.surface.dev_id].device)?;
                self.painter.mode = mode;
                #[cfg(feature = "debug-damage")]
                {
                    self.overlay = DamageOverlay::default();
                }
                return Ok(Presentation::Retry);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(NativeError::Render("surface validation failed".into()));
            }
        };
        #[cfg(feature = "debug-damage")]
        {
            self.overlay.expire(Instant::now());
            if let Some(frame) = frame {
                self.paint_frame(frame)?;
                if let Some(region) = self.stats.damage {
                    self.overlay.flash(region, Instant::now());
                }
            } else {
                self.stats = RenderStats::default();
            }
        }
        #[cfg(not(feature = "debug-damage"))]
        self.paint_frame(frame)?;
        let device = &self.context.devices[self.surface.dev_id];
        // Reuse RenderSurface's target slots instead of retaining its original,
        // unused full-size allocation alongside our persistent target.
        self.surface.target_texture = self.painter.target().clone();
        self.surface.target_view = self.surface.target_texture.create_view(&Default::default());
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
        #[cfg(feature = "debug-damage")]
        self.overlay.draw(
            &device.device,
            &mut encoder,
            &view,
            self.surface.config.format,
            self.surface.config.width,
            self.surface.config.height,
            self.window.scale_factor(),
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

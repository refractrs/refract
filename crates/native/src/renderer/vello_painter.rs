//! Offscreen rasterization shared by window presentation, pixel tests and profiling.
use super::{
    vello_scene::{color, Backdrop, Encoder, SceneResources},
    Damage, DamageTracker, Frame, PixelRect,
};
use crate::{NativeError, Result};
use vello::{
    kurbo::{Affine, Rect},
    peniko::Fill,
    wgpu, AaConfig, AaSupport, RenderParams, RendererOptions, Scene,
};

/// Rasterize the whole scene or retain pixels and repaint damage. Full rendering
/// remains available as a correctness oracle and for performance comparisons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMode {
    Full,
    #[default]
    Incremental,
}

pub(super) struct Painter {
    renderer: vello::Renderer,
    scene: Scene,
    raster: Scene,
    resources: SceneResources,
    effects: super::vello_effects::Effects,
    damage: DamageTracker,
    target: Option<wgpu::Texture>,
    scratch: Option<wgpu::Texture>,
    pub mode: RenderMode,
}

/// Work submitted by the most recent successful rasterization (excludes presentation).
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub commands: usize,
    pub pixels: u64,
    /// Actual physical region rasterized, including any full-render fallback.
    #[cfg(feature = "debug-damage")]
    pub damage: Option<PixelRect>,
}

impl Painter {
    pub fn new(device: &wgpu::Device) -> Result<Self> {
        Ok(Self {
            renderer: vello::Renderer::new(
                device,
                RendererOptions {
                    use_cpu: false,
                    antialiasing_support: AaSupport::area_only(),
                    ..Default::default()
                },
            )
            .map_err(render_error)?,
            scene: Scene::new(),
            raster: Scene::new(),
            resources: SceneResources::default(),
            effects: Default::default(),
            damage: DamageTracker::default(),
            target: None,
            scratch: None,
            mode: RenderMode::default(),
        })
    }

    pub fn target(&self) -> &wgpu::Texture {
        self.target.as_ref().expect("paint before presenting")
    }

    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
        width: u32,
        height: u32,
    ) -> Result<RenderStats> {
        if self
            .target
            .as_ref()
            .is_none_or(|t| t.width() != width || t.height() != height)
        {
            self.target = Some(texture(device, width, height));
            self.scratch = None;
            self.damage.invalidate();
        }
        let mut damage = match self.mode {
            RenderMode::Full => Damage::Full,
            RenderMode::Incremental => self.damage.plan(frame, width, height),
        };
        if let Damage::Region(region) = damage {
            let area = u64::from(region.width) * u64::from(region.height);
            let full_area = u64::from(width) * u64::from(height);
            // Large damage cannot repay selection, translation and the extra copy.
            if area >= full_area - full_area / 4 {
                damage = Damage::Full;
            }
        }
        let region = match damage {
            Damage::None => {
                self.damage.commit(frame, width, height);
                return Ok(RenderStats::default());
            }
            Damage::Full => PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            },
            Damage::Region(region) => region,
        };
        let selected;
        let commands = if matches!(damage, Damage::Region(_)) {
            selected = self.damage.commands_for_region(frame, region);
            &selected
        } else {
            &frame.commands
        };
        self.effects.begin_frame(&mut self.renderer);
        self.resources.begin_frame();
        self.scene.reset();
        let extent = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut backdrop = Backdrop::default();
        let has_backdrop = commands.iter().any(|c| matches!(c, super::DrawCommand::PushEffects { backdrop_blur, .. } if *backdrop_blur > 0.0));
        // Keep the original full encoder path, including its backdrop setup, as
        // the baseline. Regional updates never contain filters.
        backdrop.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(frame.background),
            None,
            &extent,
        );
        Encoder {
            resources: &mut self.resources,
            effects: &mut self.effects,
            renderer: &mut self.renderer,
            device,
            queue,
            scale_factor: frame.scale_factor,
            has_backdrop,
        }
        .encode(&mut commands.iter(), &mut self.scene, &backdrop, extent)?;
        let partial = matches!(damage, Damage::Region(_));
        if partial {
            if self
                .scratch
                .as_ref()
                .is_none_or(|t| t.width() != region.width || t.height() != region.height)
            {
                self.scratch = Some(texture(device, region.width, region.height));
            }
            self.raster.reset();
            self.raster.append(
                &self.scene,
                Some(Affine::translate((
                    -f64::from(region.x),
                    -f64::from(region.y),
                ))),
            );
        }
        let target = if partial {
            self.scratch.as_ref().unwrap()
        } else {
            self.target()
        };
        let view = target.create_view(&Default::default());
        self.renderer
            .render_to_texture(
                device,
                queue,
                if partial { &self.raster } else { &self.scene },
                &view,
                &RenderParams {
                    base_color: color(frame.background),
                    width: region.width,
                    height: region.height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(render_error)?;
        if partial {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Refract damage copy"),
            });
            let mut destination = self.target().as_image_copy();
            destination.origin = wgpu::Origin3d {
                x: region.x,
                y: region.y,
                z: 0,
            };
            encoder.copy_texture_to_texture(
                self.scratch.as_ref().unwrap().as_image_copy(),
                destination,
                wgpu::Extent3d {
                    width: region.width,
                    height: region.height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
        }
        let stats = RenderStats {
            commands: commands.len(),
            pixels: u64::from(region.width) * u64::from(region.height),
            #[cfg(feature = "debug-damage")]
            damage: Some(region),
        };
        // Full mode does no comparison/snapshot work, matching the original path.
        if self.mode == RenderMode::Incremental {
            self.damage.commit(frame, width, height);
        } else {
            self.damage.invalidate();
        }
        Ok(stats)
    }
}

fn texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Refract retained pixels"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn render_error(error: impl std::fmt::Display) -> NativeError {
    NativeError::Render(error.to_string())
}

#[cfg(test)]
#[path = "vello_incremental_tests.rs"]
mod tests;

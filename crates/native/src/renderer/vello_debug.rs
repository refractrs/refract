//! Presentation-only damage flash. Never writes into the retained scene texture.
use super::PixelRect;
use std::time::{Duration, Instant};
use vello::wgpu;

const FLASH_DURATION: Duration = Duration::from_millis(160);

#[derive(Default)]
pub(super) struct DamageOverlay {
    flash: Option<(PixelRect, Instant)>,
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl DamageOverlay {
    pub fn flash(&mut self, region: PixelRect, now: Instant) {
        self.flash = Some((region, now + FLASH_DURATION));
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.flash.map(|(_, deadline)| deadline)
    }

    pub fn clear(&mut self) {
        self.flash = None;
    }

    pub fn expire(&mut self, now: Instant) {
        if self.deadline().is_some_and(|deadline| deadline <= now) {
            self.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        scale: f64,
    ) {
        let Some((region, _)) = self.flash else {
            return;
        };
        let Some(edges) = outline(region, width, height, (2.0 * scale).ceil().max(1.0) as u32)
        else {
            return;
        };
        if self
            .pipeline
            .as_ref()
            .is_none_or(|(current, _)| *current != format)
        {
            self.pipeline = Some((format, pipeline(device, format)));
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Refract damage outline"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline.as_ref().unwrap().1);
        for edge in edges {
            pass.set_scissor_rect(edge.x, edge.y, edge.width, edge.height);
            pass.draw(0..3, 0..1);
        }
    }
}

fn outline(region: PixelRect, width: u32, height: u32, thickness: u32) -> Option<[PixelRect; 4]> {
    let right = region.x.saturating_add(region.width).min(width);
    let bottom = region.y.saturating_add(region.height).min(height);
    if right <= region.x || bottom <= region.y {
        return None;
    }
    let w = right - region.x;
    let h = bottom - region.y;
    let tx = thickness.max(1).min(w);
    let ty = thickness.max(1).min(h);
    Some([
        PixelRect {
            x: region.x,
            y: region.y,
            width: w,
            height: ty,
        },
        PixelRect {
            x: region.x,
            y: bottom - ty,
            width: w,
            height: ty,
        },
        PixelRect {
            x: region.x,
            y: region.y,
            width: tx,
            height: h,
        },
        PixelRect {
            x: right - tx,
            y: region.y,
            width: tx,
            height: h,
        },
    ])
}

fn pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Refract damage outline"),
        source: wgpu::ShaderSource::Wgsl(include_str!("damage.wgsl").into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Refract damage outline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_expires_without_extending_on_unchanged_presentations() {
        let now = Instant::now();
        let mut overlay = DamageOverlay::default();
        let region = PixelRect {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        overlay.flash(region, now);
        overlay.expire(now + Duration::from_millis(100));
        assert_eq!(overlay.deadline(), Some(now + FLASH_DURATION));
        overlay.expire(now + FLASH_DURATION);
        assert_eq!(overlay.deadline(), None);
        overlay.flash(region, now);
        overlay.clear();
        assert_eq!(overlay.deadline(), None);
    }

    #[test]
    fn outlines_stay_inside_damage_and_handle_small_or_resized_regions() {
        let region = PixelRect {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        let edges = outline(region, 35, 45, 4).unwrap();
        for edge in edges {
            assert!(edge.x >= 10 && edge.y >= 20);
            assert!(edge.x + edge.width <= 35 && edge.y + edge.height <= 45);
        }
        assert!(outline(region, 5, 5, 4).is_none());
        let tiny = PixelRect {
            width: 1,
            height: 1,
            ..region
        };
        assert_eq!(outline(tiny, 100, 100, 4), Some([tiny; 4]));
    }
}

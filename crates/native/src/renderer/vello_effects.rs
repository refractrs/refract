//! Bounded GPU Gaussian filtering. Vello textures use straight sRGB alpha;
//! convolution is performed in premultiplied sRGB to avoid dark fringes.
use crate::{NativeError, Result};
use vello::{
    kurbo::{Affine, Rect},
    peniko::{Color, ImageData},
    wgpu::{self, util::DeviceExt},
    AaConfig, RenderParams, Scene,
};

#[derive(Default)]
pub(super) struct Effects {
    pipeline: Option<wgpu::ComputePipeline>,
    images: Vec<ImageData>,
}

impl Effects {
    pub fn begin_frame(&mut self, renderer: &mut vello::Renderer) {
        for image in self.images.drain(..) {
            renderer.unregister_texture(image);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn blur(
        &mut self,
        renderer: &mut vello::Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        bounds: Rect,
        sigma: f64,
        mirror_edges: bool,
    ) -> Result<Scene> {
        // Downsample large blurs instead of increasing kernel cost or allocating
        // unbounded textures. At most 2048² pixels and 25 taps per pass.
        let limit = device.limits().max_texture_dimension_2d.min(2048);
        let resolution = (4.0 / sigma)
            .min(1.0)
            .min(f64::from(limit) / bounds.width())
            .min(f64::from(limit) / bounds.height());
        // Keep a few source samples for mirrored backdrops: collapsing adjacent
        // shapes into a single antialiased pixel can lose their opaque coverage.
        let minimum = if mirror_edges { 4.0 } else { 1.0 };
        let width = (bounds.width() * resolution)
            .ceil()
            .clamp(minimum, f64::from(limit)) as u32;
        let height = (bounds.height() * resolution)
            .ceil()
            .clamp(minimum, f64::from(limit)) as u32;
        // Map the complete source box onto the rounded pixel dimensions. In
        // particular, a large blur reduces a panel to very few pixels; leaving
        // fractional coverage would incorrectly fade it.
        let resolution_x = f64::from(width) / bounds.width();
        let resolution_y = f64::from(height) / bounds.height();
        if !mirror_edges
            && [bounds.x0, bounds.y0, bounds.x1, bounds.y1]
                .into_iter()
                .any(|v| v.abs() > f64::from(f32::MAX))
        {
            // Vello encodes f32 coordinates. At this scale a finite viewport's
            // contribution to the Gaussian is below RGBA8 precision.
            return Ok(Scene::new());
        }
        let source = texture(device, width, height);
        let intermediate = texture(device, width, height);
        let output = texture(device, width, height);
        let source_view = source.create_view(&Default::default());
        let intermediate_view = intermediate.create_view(&Default::default());
        let output_view = output.create_view(&Default::default());
        let mut raster = Scene::new();
        raster.append(
            scene,
            Some(
                Affine::scale_non_uniform(resolution_x, resolution_y)
                    * Affine::translate((-bounds.x0, -bounds.y0)),
            ),
        );
        renderer
            .render_to_texture(
                device,
                queue,
                &raster,
                &source_view,
                &RenderParams {
                    base_color: Color::TRANSPARENT,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(|e| NativeError::Render(e.to_string()))?;

        let pipeline = self.pipeline.get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Refract Gaussian blur"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Refract Gaussian blur"),
                layout: None,
                module: &module,
                entry_point: Some("blur"),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Refract blur passes"),
        });
        for (input, output, horizontal) in [
            (&source_view, &intermediate_view, 1_u32),
            (&intermediate_view, &output_view, 0_u32),
        ] {
            let mut bytes = Vec::with_capacity(16);
            let axis_resolution = if horizontal != 0 {
                resolution_x
            } else {
                resolution_y
            };
            bytes.extend_from_slice(&((sigma * axis_resolution).min(16.0) as f32).to_ne_bytes());
            bytes.extend_from_slice(&horizontal.to_ne_bytes());
            bytes.extend_from_slice(&u32::from(mirror_edges).to_ne_bytes());
            bytes.extend_from_slice(&[0_u8; 4]);
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Refract blur parameters"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Refract blur bindings"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(output),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Refract blur"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        queue.submit([encoder.finish()]);
        let image = renderer.register_texture(output);
        let mut filtered = Scene::new();
        filtered.draw_image(
            &image,
            Affine::translate((bounds.x0, bounds.y0))
                * Affine::scale_non_uniform(1.0 / resolution_x, 1.0 / resolution_y),
        );
        self.images.push(image);
        Ok(filtered)
    }
}

fn texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Refract blur surface"),
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
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[cfg(test)]
#[path = "vello_effects_tests.rs"]
mod tests;

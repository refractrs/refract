//! Render application WGSL into textures that Vello composites in paint order.

use crate::{NativeError, Result};
use refract_core::{Shader, ShaderImage, Size};
use std::{collections::HashMap, sync::Arc, time::Duration};
use vello::{
    peniko::ImageData,
    wgpu::{self, util::DeviceExt},
};

#[derive(Default)]
pub(super) struct Shaders {
    pipelines: HashMap<Arc<str>, wgpu::RenderPipeline>,
    surfaces: Vec<Surface>,
    cursor: usize,
}
struct Surface {
    texture: wgpu::Texture,
    image: ImageData,
    frame: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    input: wgpu::Texture,
    bindings: wgpu::BindGroup,
    shader: Shader,
    size: Size,
    elapsed: Duration,
}
impl Shaders {
    pub fn begin_frame(&mut self) {
        self.cursor = 0;
    }

    pub fn end_frame(&mut self, renderer: &mut vello::Renderer) {
        for surface in self.surfaces.drain(self.cursor..) {
            renderer.unregister_texture(surface.image);
        }
        self.pipelines
            .retain(|source, _| self.surfaces.iter().any(|s| s.shader.source() == source));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        renderer: &mut vello::Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shader: &Shader,
        size: Size,
        scale: f64,
        elapsed: Duration,
    ) -> Result<ImageData> {
        let limit = device.limits().max_texture_dimension_2d.min(8192);
        let width = (size.width * scale).ceil().clamp(1.0, f64::from(limit)) as u32;
        let height = (size.height * scale).ceil().clamp(1.0, f64::from(limit)) as u32;
        if !self.pipelines.contains_key(shader.source()) {
            self.pipelines
                .insert(shader.source().clone(), pipeline(device, shader)?);
        }
        let pipeline = &self.pipelines[shader.source()];
        let recreate = self.surfaces.get(self.cursor).is_none_or(|s| {
            s.texture.width() != width
                || s.texture.height() != height
                || s.shader.source() != shader.source()
                || dimensions(s.shader.image_input()) != dimensions(shader.image_input())
        });
        if recreate {
            let surface = Surface::new(
                renderer, device, queue, pipeline, shader, size, width, height, elapsed,
            )?;
            if self.cursor < self.surfaces.len() {
                let old = std::mem::replace(&mut self.surfaces[self.cursor], surface);
                renderer.unregister_texture(old.image);
            } else {
                self.surfaces.push(surface);
            }
        }
        let surface = &mut self.surfaces[self.cursor];
        self.cursor += 1;
        if recreate
            || surface.shader != *shader
            || surface.size != size
            || surface.elapsed != elapsed
        {
            queue.write_buffer(
                &surface.frame,
                0,
                &Shader::frame_bytes(size, width, height, elapsed),
            );
            queue.write_buffer(
                &surface.uniforms,
                0,
                &shader
                    .uniform_bytes()
                    .map_err(|e| NativeError::Render(e.to_string()))?,
            );
            if !recreate && surface.shader.image_input() != shader.image_input() {
                upload(queue, &surface.input, shader.image_input());
            }
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Refract shader frame"),
            });
            let view = surface.texture.create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Refract shader"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &surface.bindings, &[]);
                pass.draw(0..3, 0..1);
            }
            queue.submit([encoder.finish()]);
            renderer.mark_override_image_dirty(&surface.image);
            surface.shader = shader.clone();
            surface.size = size;
            surface.elapsed = elapsed;
        }
        Ok(surface.image.clone())
    }
}

fn pipeline(device: &wgpu::Device, shader: &Shader) -> Result<wgpu::RenderPipeline> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Refract application shader"),
        source: wgpu::ShaderSource::Wgsl(shader.source().as_ref().into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Refract shader bindings"),
        entries: &[
            uniform_layout(0),
            uniform_layout(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                count: None,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                count: None,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Refract shader layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Refract application shader"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("refract_vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("refract_fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(NativeError::Render(error.to_string()));
    }
    Ok(pipeline)
}
fn uniform_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        count: None,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    }
}
impl Surface {
    #[allow(clippy::too_many_arguments)]
    fn new(
        renderer: &mut vello::Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        shader: &Shader,
        size: Size,
        width: u32,
        height: u32,
        elapsed: Duration,
    ) -> Result<Self> {
        let texture = texture(
            device,
            width,
            height,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let image = renderer.register_texture(texture.clone());
        let frame = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Refract shader frame"),
            contents: &Shader::frame_bytes(size, width, height, elapsed),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Refract shader uniforms"),
            contents: &shader
                .uniform_bytes()
                .map_err(|e| NativeError::Render(e.to_string()))?,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let (iw, ih) = dimensions(shader.image_input());
        let input = texture_fn(device, iw, ih);
        upload(queue, &input, shader.image_input());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Refract shader bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &input.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Ok(Self {
            texture,
            image,
            frame,
            uniforms,
            input,
            bindings,
            shader: shader.clone(),
            size,
            elapsed,
        })
    }
}
fn dimensions(image: Option<&ShaderImage>) -> (u32, u32) {
    image.map_or((1, 1), |i| (i.width(), i.height()))
}
fn texture_fn(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    texture(device, width, height, wgpu::TextureUsages::COPY_DST)
}
fn texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Refract shader surface"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: usage | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, image: Option<&ShaderImage>) {
    let (width, height) = dimensions(image);
    queue.write_texture(
        texture.as_image_copy(),
        image.map_or(&[0, 0, 0, 0], ShaderImage::pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

//! Pixel tests exercise the production encoder and GPU passes without a window.
use super::*;
use crate::renderer::{
    vello_scene::{Backdrop, Encoder, SceneResources},
    DrawCommand, Rect as BoxRect,
};
use refract_core::Color as Paint;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    effects: Effects,
    resources: SceneResources,
}

impl Gpu {
    fn new() -> Self {
        let mut context = vello::util::RenderContext::new();
        let id = pollster::block_on(context.device(None)).expect("GPU device required");
        let device = context.devices[id].device.clone();
        let queue = context.devices[id].queue.clone();
        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        )
        .unwrap();
        Self {
            device,
            queue,
            renderer,
            effects: Effects::default(),
            resources: SceneResources::default(),
        }
    }

    fn pixels(&mut self, commands: &[DrawCommand], scale: f64) -> Vec<u8> {
        self.effects.begin_frame(&mut self.renderer);
        let size = (96.0 * scale) as u32;
        let mut scene = Scene::new();
        Encoder {
            resources: &mut self.resources,
            effects: &mut self.effects,
            renderer: &mut self.renderer,
            device: &self.device,
            queue: &self.queue,
            scale_factor: scale,
            has_backdrop: true,
        }
        .encode(
            &mut commands.iter(),
            &mut scene,
            &Backdrop::default(),
            Rect::new(0.0, 0.0, f64::from(size), f64::from(size)),
        )
        .unwrap();
        let target = texture(&self.device, size, size);
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                &scene,
                &target.create_view(&Default::default()),
                &RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: size,
                    height: size,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .unwrap();
        let stride = (size * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur test readback"),
            size: u64::from(stride * size),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        buffer
            .slice(..)
            .get_mapped_range()
            .unwrap()
            .chunks(stride as usize)
            .flat_map(|row| row[..(size * 4) as usize].iter().copied())
            .collect()
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> BoxRect {
    BoxRect {
        x,
        y,
        width,
        height,
    }
}
fn fill(rect: BoxRect, color: Paint) -> DrawCommand {
    DrawCommand::Fill {
        rect,
        radius: 0.0.into(),
        color,
    }
}
fn effect(blur: f32, backdrop_blur: f32, opacity: f32) -> DrawCommand {
    DrawCommand::PushEffects {
        rect: rect(16.0, 16.0, 64.0, 64.0),
        radius: 12.0.into(),
        blur,
        backdrop_blur,
        opacity,
    }
}
fn pixel(data: &[u8], x: usize, y: usize, scale: usize) -> &[u8] {
    let i = ((y * scale) * (96 * scale) + x * scale) * 4;
    &data[i..i + 4]
}

#[test]
#[ignore = "requires a GPU: cargo test -p refract-native gpu_blur -- --ignored"]
fn gpu_blur_composites_subtrees_backdrops_clips_and_alpha() {
    let mut gpu = Gpu::new();
    let red = Paint(255, 0, 0, 255);
    let block = rect(32.0, 32.0, 32.0, 32.0);
    for scale in [1, 2] {
        let blurred = gpu.pixels(
            &[
                effect(4.0, 0.0, 1.0),
                fill(block, red),
                DrawCommand::PopLayer,
            ],
            scale as f64,
        );
        let edge = pixel(&blurred, 30, 48, scale);
        assert!(
            edge[0] > 245 && edge[3] > 40 && edge[3] < 130,
            "premultiplied blur edge: {edge:?}"
        );
        assert_eq!(pixel(&blurred, 10, 48, scale)[3], 0);
        let grouped = gpu.pixels(
            &[
                effect(4.0, 0.0, 0.5),
                fill(rect(32.0, 32.0, 16.0, 32.0), red),
                fill(rect(48.0, 32.0, 16.0, 32.0), red),
                DrawCommand::PopLayer,
            ],
            scale as f64,
        );
        assert!(
            (120..=130).contains(&pixel(&grouped, 48, 48, scale)[3]),
            "children blur together, opacity applied once"
        );
    }

    let backdrop = gpu.pixels(
        &[
            fill(rect(0.0, 0.0, 48.0, 96.0), Paint::BLACK),
            fill(rect(48.0, 0.0, 48.0, 96.0), Paint::WHITE),
            effect(0.0, 5.0, 1.0),
            fill(rect(40.0, 40.0, 16.0, 16.0), red),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert!(
        (60..190).contains(&pixel(&backdrop, 47, 30, 1)[0]),
        "backdrop edge softens"
    );
    assert_eq!(
        pixel(&backdrop, 47, 8, 1),
        &[0, 0, 0, 255],
        "outside surface stays sharp"
    );
    assert_eq!(
        pixel(&backdrop, 40, 48, 1),
        &[255, 0, 0, 255],
        "foreground edge stays sharp"
    );

    let clipped = gpu.pixels(
        &[
            DrawCommand::PushLayer {
                clip: rect(0.0, 0.0, 48.0, 96.0),
                radius: 0.0.into(),
                opacity: 1.0,
            },
            effect(6.0, 0.0, 1.0),
            fill(block, red),
            DrawCommand::PopLayer,
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert!(pixel(&clipped, 47, 48, 1)[3] > 150);
    assert_eq!(
        pixel(&clipped, 49, 48, 1)[3],
        0,
        "ancestor clips constrain blur output"
    );

    let offscreen = gpu.pixels(
        &[
            effect(6.0, 0.0, 1.0),
            fill(rect(-16.0, 20.0, 16.0, 50.0), red),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert!(
        pixel(&offscreen, 1, 48, 1)[3] > 50,
        "offscreen content contributes to visible blur"
    );
    let nested = gpu.pixels(
        &[
            effect(4.0, 0.0, 1.0),
            effect(4.0, 0.0, 1.0),
            fill(block, red),
            DrawCommand::PopLayer,
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    let equivalent = gpu.pixels(
        &[
            effect(32.0_f32.sqrt(), 0.0, 1.0),
            fill(block, red),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert!(
        pixel(&nested, 30, 48, 1)[3].abs_diff(pixel(&equivalent, 30, 48, 1)[3]) < 15,
        "nested Gaussian variances add"
    );

    let corners = gpu.pixels(
        &[
            fill(rect(0.0, 0.0, 18.0, 96.0), Paint::BLACK),
            fill(rect(18.0, 0.0, 78.0, 96.0), Paint::WHITE),
            effect(0.0, 5.0, 1.0),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert_eq!(
        pixel(&corners, 17, 17, 1),
        &[0, 0, 0, 255],
        "rounded corner excludes backdrop output"
    );
    assert!(
        pixel(&corners, 17, 48, 1)[0] > 30,
        "inside the surface the backdrop softens"
    );

    let isolated = gpu.pixels(
        &[
            fill(rect(0.0, 0.0, 48.0, 96.0), Paint::BLACK),
            fill(rect(48.0, 0.0, 48.0, 96.0), Paint::WHITE),
            DrawCommand::PushLayer {
                clip: rect(0.0, 0.0, 96.0, 96.0),
                radius: 0.0.into(),
                opacity: 0.5,
            },
            effect(0.0, 6.0, 1.0),
            DrawCommand::PopLayer,
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert_eq!(
        pixel(&isolated, 47, 48, 1),
        &[0, 0, 0, 255],
        "backdrop cannot sample outside opacity boundary"
    );
    assert_eq!(pixel(&isolated, 48, 48, 1), &[255, 255, 255, 255]);

    let large = gpu.pixels(
        &[
            effect(1000.0, 0.0, 1.0),
            fill(block, red),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert!(
        pixel(&large, 48, 48, 1)[3] < 10,
        "large blur is bounded and spreads energy"
    );
    let huge_backdrop = gpu.pixels(
        &[
            fill(rect(0.0, 0.0, 48.0, 96.0), Paint::BLACK),
            fill(rect(48.0, 0.0, 48.0, 96.0), Paint::WHITE),
            effect(0.0, 1000.0, 1.0),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    for x in [47, 48] {
        assert!(
            (100..155).contains(&pixel(&huge_backdrop, x, 48, 1)[0]),
            "a very large backdrop blur averages the panel instead of fading its coverage"
        );
    }
    let extreme = gpu.pixels(
        &[
            effect(f32::MAX, 0.0, 1.0),
            fill(block, red),
            DrawCommand::PopLayer,
        ],
        1.0,
    );
    assert_eq!(
        pixel(&extreme, 48, 48, 1)[3],
        0,
        "extreme finite blur stays safe"
    );
    let reset = gpu.pixels(&[fill(block, red)], 1.0);
    assert_eq!(
        pixel(&reset, 30, 48, 1)[3],
        0,
        "removing blur restores a sharp edge"
    );
}

#[test]
#[ignore = "requires a GPU"]
fn gpu_animation_transforms_preserve_clips_and_group_opacity() {
    let mut gpu=Gpu::new();
    let commands=vec![
        DrawCommand::PushLayer{clip:rect(0.0,0.0,48.0,96.0),radius:0.0.into(),opacity:1.0},
        DrawCommand::PushTransform(refract_core::Transform{offset_x:32.0,offset_y:16.0,..Default::default()}.matrix(0.0,0.0,32.0,32.0)),
        DrawCommand::PushOpacity(0.5),
        fill(rect(0.0,0.0,32.0,32.0),Paint::rgb(255,0,0)),
        DrawCommand::PopLayer,DrawCommand::PopLayer,DrawCommand::PopLayer,
    ];
    for scale in [1,2] {
        let data=gpu.pixels(&commands,scale as f64);
        assert_eq!(pixel(&data,10,10,scale),&[0,0,0,0]);
        let inside=pixel(&data,40,24,scale);
        assert!(inside[0]>=245 && inside[3]>=126 && inside[3]<=129,"{inside:?}");
        assert_eq!(pixel(&data,56,24,scale),&[0,0,0,0]);
    }
}

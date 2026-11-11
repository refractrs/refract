//! Run GPU checks explicitly; ordinary CI need not have a GPU.
use super::*;
use crate::{
    renderer::{DrawCommand, Point, Rect as BoxRect},
    text::{TextSystem, Typography},
};
use refract_core::{Color, CornerRadii};
use std::time::Instant;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    full: Painter,
    incremental: Painter,
}
impl Gpu {
    fn new() -> Self {
        let mut context = vello::util::RenderContext::new();
        let id = pollster::block_on(context.device(None)).expect("GPU device required");
        let device = context.devices[id].device.clone();
        let queue = context.devices[id].queue.clone();
        eprintln!("adapter: {:?}", context.devices[id].adapter().get_info());
        let mut full = Painter::new(&device).unwrap();
        full.mode = RenderMode::Full;
        let incremental = Painter::new(&device).unwrap();
        Self {
            device,
            queue,
            full,
            incremental,
        }
    }

    fn compare(&mut self, frame: &Frame, width: u32, height: u32) -> RenderStats {
        self.full
            .paint(&self.device, &self.queue, frame, width, height)
            .unwrap();
        let stats = self
            .incremental
            .paint(&self.device, &self.queue, frame, width, height)
            .unwrap();
        let full = readback(&self.device, &self.queue, self.full.target());
        let incremental = readback(&self.device, &self.queue, self.incremental.target());
        let max = full
            .iter()
            .zip(&incremental)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        // Independent Vello instances can differ by two channel levels at
        // antialiased edges even when BOTH repaint the entire frame. Keep a
        // small absolute tolerance across every pixel; missing/stale paint
        // still fails rather than being hidden by a whole-image average.
        assert!(
            max <= 2,
            "incremental differs from full: max channel error {max}"
        );
        stats
    }
}

fn readback(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("incremental readback"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    buffer
        .slice(..)
        .get_mapped_range()
        .unwrap()
        .chunks(stride as usize)
        .flat_map(|r| r[..width as usize * 4].iter().copied())
        .collect()
}
fn rect(x: f32, y: f32, width: f32, height: f32) -> BoxRect {
    BoxRect {
        x,
        y,
        width,
        height,
    }
}
fn fill(rect: BoxRect, color: Color) -> DrawCommand {
    DrawCommand::Fill {
        rect,
        radius: 7.0.into(),
        color,
    }
}
fn frame(commands: Vec<DrawCommand>, scale_factor: f64) -> Frame {
    Frame {
        commands,
        background: Color(24, 26, 30, 255),
        scale_factor,
    }
}
fn text(value: &str, origin: Point, italic: bool) -> Vec<DrawCommand> {
    let mut system = TextSystem::default();
    let typography = Typography {
        size: 24.0,
        italic,
        ..Default::default()
    };
    let mut block = system.shape_label(value, &typography, false);
    block.measure(Some(500.0), refract_core::Alignment::Start);
    block
        .glyph_runs()
        .iter()
        .map(|run| DrawCommand::Glyphs {
            origin,
            color: Color::WHITE,
            run: run.clone(),
        })
        .collect()
}

#[test]
#[ignore = "requires GPU: cargo test -p refract-native gpu_incremental -- --ignored --nocapture"]
fn gpu_incremental_matches_full_rendering() {
    let mut gpu = Gpu::new();
    for scale in [1.0, 1.5, 2.0] {
        let (w, h) = ((320.0 * scale) as u32, (240.0 * scale) as u32);
        let mut f = frame(
            vec![
                fill(rect(0.0, 0.0, 320.0, 240.0), Color(30, 80, 110, 255)),
                DrawCommand::PushLayer {
                    clip: rect(9.5, 12.5, 270.0, 200.0),
                    radius: 19.0.into(),
                    opacity: 0.65,
                },
                DrawCommand::BoxShadow {
                    rect: rect(30.25, 40.5, 80.0, 60.0),
                    radius: CornerRadii::default().top_left(18.0),
                    shadow: refract_core::BoxShadow::new(-9.0, 6.0, 12.0, Color::BLACK),
                },
                fill(rect(30.25, 40.5, 80.0, 60.0), Color(240, 80, 60, 180)),
                DrawCommand::PushInsetLayer {
                    rect: rect(70.0, 65.0, 110.0, 100.0),
                    radius: 20.0.into(),
                    insets: refract_core::Insets::all(8.0),
                },
                fill(rect(65.0, 60.0, 120.0, 120.0), Color(80, 240, 60, 130)),
                DrawCommand::PopLayer,
                DrawCommand::PopLayer,
            ],
            scale,
        );
        gpu.compare(&f, w, h);
        assert_eq!(gpu.compare(&f, w, h).pixels, 0);
        f.commands[3] = fill(rect(50.75, 48.25, 70.0, 55.0), Color(80, 60, 240, 100));
        assert!(gpu.compare(&f, w, h).pixels < u64::from(w * h));
        if let DrawCommand::BoxShadow { rect, .. } = &mut f.commands[2] {
            rect.x += 25.0;
        }
        gpu.compare(&f, w, h);
        f.commands.swap(2, 3);
        gpu.compare(&f, w, h);
        f.commands.remove(2);
        gpu.compare(&f, w, h);
        f.background = Color(0, 0, 0, 0);
        gpu.compare(&f, w, h);
        gpu.compare(&f, w + 17, h + 11);
        f.commands
            .extend(text("Italic Ágj", Point { x: 20.0, y: 145.0 }, true));
        gpu.compare(&f, w, h);
        for command in f.commands.iter_mut().skip(7) {
            if let DrawCommand::Glyphs { origin, color, .. } = command {
                origin.x += 12.5;
                *color = Color(255, 60, 90, 160);
            }
        }
        assert!(gpu.compare(&f, w, h).pixels < u64::from(w * h));
        // A backdrop depends on earlier content, even if the effect is unchanged.
        f.commands.extend([
            DrawCommand::PushEffects {
                rect: rect(20.0, 20.0, 80.0, 80.0),
                radius: 10.0.into(),
                blur: 2.0,
                backdrop_blur: 3.0,
                opacity: 0.8,
            },
            fill(rect(30.0, 30.0, 25.0, 25.0), Color::WHITE),
            DrawCommand::PopLayer,
        ]);
        gpu.compare(&f, w, h);
        f.commands[0] = fill(rect(0.0, 0.0, 320.0, 240.0), Color(80, 30, 100, 255));
        assert_eq!(gpu.compare(&f, w, h).pixels, u64::from(w * h));
    }
}

#[test]
#[ignore = "requires GPU: cargo test -p refract-native gpu_incremental -- --ignored --nocapture"]
fn gpu_incremental_handles_images_gradients_and_recovery() {
    let mut gpu = Gpu::new();
    let image = std::sync::Arc::new(crate::renderer::RasterImage {
        id: crate::renderer::ImageId(100),
        width: 2,
        height: 2,
        rgba: std::sync::Arc::from([
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 255, 255, 255, 0,
        ]),
    });
    let mut f = frame(
        vec![
            DrawCommand::Image {
                rect: rect(35.25, 25.75, 90.0, 80.0),
                image: image.clone(),
            },
            fill(rect(80.0, 60.0, 70.0, 60.0), Color(200, 90, 30, 130)),
            DrawCommand::LinearGradient {
                rect: rect(120.5, 80.25, 60.0, 100.0),
                radius: 15.0.into(),
                gradient: refract_core::LinearGradient::new(45.0, Color::WHITE, Color::BLACK),
            },
        ],
        1.5,
    );
    gpu.compare(&f, 480, 360);
    for i in 0..24 {
        f.commands[0] = DrawCommand::Image {
            rect: rect(35.25 + i as f32 * 2.25, 25.75, 90.0, 80.0),
            image: image.clone(),
        };
        assert!(gpu.compare(&f, 480, 360).pixels < 480 * 360);
    }
    f.commands
        .insert(1, fill(rect(20.5, 30.5, 2.0, 26.0), Color::WHITE));
    assert!(gpu.compare(&f, 480, 360).pixels < 480 * 360);
    f.commands.remove(1);
    assert!(gpu.compare(&f, 480, 360).pixels < 480 * 360);
    f.commands
        .extend(text("before", Point { x: 30.0, y: 180.0 }, false));
    gpu.compare(&f, 480, 360);
    f.commands.truncate(3);
    f.commands
        .extend(text("after!", Point { x: 30.0, y: 180.0 }, false));
    assert!(gpu.compare(&f, 480, 360).pixels < 480 * 360);
    gpu.incremental = Painter::new(&gpu.device).unwrap();
    assert_eq!(gpu.compare(&f, 480, 360).pixels, 480 * 360);
    gpu.incremental.mode = RenderMode::Full;
    gpu.compare(&f, 480, 360);
    gpu.incremental.mode = RenderMode::Incremental;
    assert_eq!(gpu.compare(&f, 480, 360).pixels, 480 * 360);
    assert_eq!(gpu.compare(&f, 480, 360).pixels, 0);
    f.commands.clear();
    gpu.compare(&f, 480, 360);
}

#[test]
#[ignore = "release GPU profile: cargo test -p refract-native --release profile_incremental -- --ignored --nocapture"]
fn profile_incremental() {
    let mut gpu = Gpu::new();
    let (width, height) = (1920, 1080);
    let mut commands = Vec::new();
    let label = text("Retained rendering", Point::default(), false);
    for row in 0..24 {
        for column in 0..40 {
            let (x, y) = (column as f32 * 48.0, row as f32 * 45.0);
            commands.push(fill(rect(x, y, 44.0, 41.0), Color(45, 60, 80, 255)));
            // Shared shaped resources, as in the native host's retained text cache.
            for command in &label {
                let DrawCommand::Glyphs { run, .. } = command else {
                    unreachable!()
                };
                let mut run = (**run).clone();
                run.font_size = 8.0;
                for g in &mut run.glyphs {
                    g.x /= 3.0;
                    g.y /= 3.0;
                }
                commands.push(DrawCommand::Glyphs {
                    origin: Point {
                        x: x + 2.0,
                        y: y + 10.0,
                    },
                    color: Color::WHITE,
                    run: std::rc::Rc::new(run),
                });
            }
        }
    }
    let base = frame(commands, 1.0);
    eprintln!(
        "1920x1080, {} commands, 20 warmup + 100 measured frames, synchronized GPU completion",
        base.commands.len()
    );
    for workload in ["unchanged", "local fill", "local text", "all fills"] {
        for mode in [RenderMode::Full, RenderMode::Incremental] {
            let painter = if mode == RenderMode::Full {
                &mut gpu.full
            } else {
                &mut gpu.incremental
            };
            let mut samples = Vec::new();
            let mut pixels = 0;
            let mut encoded = 0;
            let mut f = base.clone();
            for i in 0..120 {
                match workload {
                    "local fill" => {
                        f.commands[984] = fill(
                            rect(576.0, 540.0, 44.0, 41.0),
                            Color(45 + (i % 2) as u8 * 100, 60, 80, 255),
                        );
                    }
                    "local text" => {
                        if let DrawCommand::Glyphs { color, .. } = &mut f.commands[985] {
                            *color = Color(100 + (i % 2) as u8 * 100, 180, 240, 255);
                        }
                    }
                    "all fills" => {
                        for command in &mut f.commands {
                            if let DrawCommand::Fill { color, .. } = command {
                                *color = Color(45 + (i % 2) as u8 * 100, 60, 80, 255);
                            }
                        }
                    }
                    _ => {}
                }
                let start = Instant::now();
                let stats = painter
                    .paint(&gpu.device, &gpu.queue, &f, width, height)
                    .unwrap();
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                if i >= 20 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                    pixels += stats.pixels;
                    encoded += stats.commands;
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "{workload:12} {mode:?}: median={:.3}ms p95={:.3}ms commands={} pixels={}",
                samples[50],
                samples[95],
                encoded / 100,
                pixels / 100
            );
        }
    }
}

#[test]
#[cfg(feature = "debug-damage")]
#[ignore = "requires GPU: cargo test -p refract-native --features debug-damage gpu_damage_overlay -- --ignored"]
fn gpu_damage_overlay_expires_without_touching_retained_pixels() {
    use crate::renderer::vello_debug::DamageOverlay;
    let mut gpu = Gpu::new();
    let frame = frame(vec![], 1.0);
    gpu.incremental
        .paint(&gpu.device, &gpu.queue, &frame, 96, 96)
        .unwrap();
    let retained = gpu.incremental.target();
    let original = readback(&gpu.device, &gpu.queue, retained);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("damage overlay test presentation"),
        size: wgpu::Extent3d {
            width: 96,
            height: 96,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let source_view = retained.create_view(&Default::default());
    let target_view = target.create_view(&Default::default());
    let blitter = wgpu::util::TextureBlitter::new(&gpu.device, target.format());
    let mut overlay = DamageOverlay::default();
    let now = Instant::now();
    overlay.flash(
        PixelRect {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        },
        now,
    );
    let present = |overlay: &mut DamageOverlay| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        blitter.copy(&gpu.device, &mut encoder, &source_view, &target_view);
        overlay.draw(
            &gpu.device,
            &mut encoder,
            &target_view,
            target.format(),
            96,
            96,
            1.5,
        );
        gpu.queue.submit([encoder.finish()]);
    };
    present(&mut overlay);
    let pixels = readback(&gpu.device, &gpu.queue, &target);
    for y in 0..96 {
        for x in 0..96 {
            let i = (y * 96 + x) * 4;
            let on_border = (10..40).contains(&x)
                && (20..60).contains(&y)
                && (!(13..37).contains(&x) || !(23..57).contains(&y));
            assert_eq!(
                &pixels[i..i + 4],
                if on_border {
                    &[255, 0, 0, 255]
                } else {
                    &original[i..i + 4]
                }
            );
        }
    }
    assert_eq!(readback(&gpu.device, &gpu.queue, retained), original);
    overlay.expire(overlay.deadline().unwrap());
    present(&mut overlay);
    assert_eq!(overlay.deadline(), None);
    assert_eq!(readback(&gpu.device, &gpu.queue, &target), original);
    assert_eq!(
        gpu.incremental
            .paint(&gpu.device, &gpu.queue, &frame, 96, 96)
            .unwrap()
            .pixels,
        0
    );
}

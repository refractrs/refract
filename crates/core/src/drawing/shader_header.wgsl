// Refract shader ABI. Positions are local logical pixels, with a top-left origin.
struct ShaderFrame {
    position: vec2<f32>,
    size: vec2<f32>,
    uv: vec2<f32>,
    time: f32,
}
struct RefractFrame {
    size: vec2<f32>,
    resolution: vec2<f32>,
    time: f32,
    padding: f32,
    padding2: vec2<f32>,
}
@group(0) @binding(0) var<uniform> refract_frame: RefractFrame;
@group(0) @binding(1) var<uniform> uniforms: Uniforms;
@group(0) @binding(2) var refract_image: texture_2d<f32>;
@group(0) @binding(3) var refract_sampler: sampler;

fn sample_image(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(refract_image, refract_sampler, uv, 0.0);
}

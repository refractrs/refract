@vertex fn refract_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
fn refract_shade(position: vec4<f32>) -> vec4<f32> {
    let uv = position.xy / refract_frame.resolution;
    return shade(ShaderFrame(uv * refract_frame.size, refract_frame.size, uv, refract_frame.time));
}
@fragment fn refract_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return refract_shade(position);
}
@fragment fn refract_fragment_premultiplied(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let color = refract_shade(position);
    return vec4<f32>(color.rgb * color.a, color.a);
}

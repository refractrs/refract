// Application code. Refract supplies ShaderFrame, uniforms and sample_image.
struct Uniforms {
    pointer: vec2<f32>,
    intensity: f32,
    image_mix: f32,
}

fn palette(t: f32) -> vec3<f32> {
    return 0.5 + 0.5 * cos(6.283185 * (vec3<f32>(t) + vec3<f32>(0.02, 0.24, 0.48)));
}

fn shade(frame: ShaderFrame) -> vec4<f32> {
    let aspect = frame.size.x / frame.size.y;
    let p = (frame.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let light = (uniforms.pointer - 0.5) * vec2<f32>(aspect, 1.0);
    let time = frame.time * 0.3;
    let distance = length(p - light);
    let warp = 0.11 * sin(distance * 13.0 - time * 4.0);
    let q = p + vec2<f32>(warp, warp * 0.7);
    let field = sin(q.x * 5.0 + time) + sin(q.y * 7.0 - time * 1.3)
        + sin(length(q + vec2<f32>(0.2 * sin(time), 0.1)) * 9.0 - time * 2.0);
    let ribbon = pow(0.5 + 0.5 * sin(field * 2.0 + time), 5.0);
    let glow = 0.13 / (0.15 + distance * 1.7);
    let color = palette(field * 0.17 + time * 0.08) * (0.18 + ribbon * 0.85 + glow);
    let edge = 1.0 - smoothstep(0.15, 1.0, length(frame.uv - 0.5));
    let procedural = color * edge * uniforms.intensity;
    let image = sample_image(frame.uv + vec2<f32>(warp, warp * 0.5));
    return vec4<f32>(mix(procedural, image.rgb * (0.5 + ribbon), uniforms.image_mix * 0.8), 1.0);
}

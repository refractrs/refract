struct Params {
    sigma: f32,
    horizontal: u32,
    mirror_edges: u32,
    padding: u32,
}
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn blur(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if any(id.xy >= size) { return; }
    let pos = vec2<i32>(id.xy);
    let sigma = max(params.sigma, 0.0001);
    let radius = min(12, i32(ceil(3.0 * sigma)));
    let direction = select(vec2<i32>(0, 1), vec2<i32>(1, 0), params.horizontal != 0u);
    var total = vec4<f32>(0.0);
    var weights = 0.0;
    for (var i = -radius; i <= radius; i += 1) {
        let weight = exp(-0.5 * pow(f32(i) / sigma, 2.0));
        var at = pos + direction * i;
        if params.mirror_edges != 0u {
            let period = 2 * vec2<i32>(size);
            at = ((at % period) + period) % period;
            at = min(at, period - 1 - at);
        }
        // Transparent samples outside the padded source, never edge clamping.
        if all(at >= vec2<i32>(0)) && all(at < vec2<i32>(size)) {
            let sample = textureLoad(source, at, 0);
            total += vec4<f32>(sample.rgb * sample.a, sample.a) * weight;
        }
        weights += weight;
    }
    total /= weights;
    let rgb = total.rgb / max(total.a, 0.000001);
    textureStore(output_image, pos, vec4<f32>(rgb, total.a));
}

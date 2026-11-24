struct BlitUniforms {
    dst_rect_ndc: vec4<f32>, // min_x, min_y, max_x, max_y in NDC [-1..1]
    tint: vec4<f32>,         // rgba multiplier (use alpha for single-pass highlighter opacity)
    src_uv_rect: vec4<f32>,  // min_u, min_v, max_u, max_v in source texture UV space
    _pad: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> u: BlitUniforms;

@group(0) @binding(1)
var src_tex: texture_2d<f32>;

@group(0) @binding(2)
var src_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[vid];
    let ndc_x = mix(u.dst_rect_ndc.x, u.dst_rect_ndc.z, uv.x);
    let ndc_y = mix(u.dst_rect_ndc.y, u.dst_rect_ndc.w, uv.y);

    var out: VsOut;
    out.pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = vec2<f32>(
        mix(u.src_uv_rect.x, u.src_uv_rect.z, uv.x),
        mix(u.src_uv_rect.y, u.src_uv_rect.w, uv.y),
    );
    return out;
}

@fragment
fn fs_alpha(in: VsOut) -> @location(0) vec4<f32> {
    let s = textureSample(src_tex, src_sampler, in.uv);
    let a = s.a * u.tint.a;
    return vec4<f32>(s.rgb * u.tint.rgb * u.tint.a, a);
}

@fragment
fn fs_multiply(in: VsOut) -> @location(0) vec4<f32> {
    // Premultiplied multiply composite: dst * src_rgb + dst * (1 - src_a)
    let s = textureSample(src_tex, src_sampler, in.uv);
    let cov = s.a * u.tint.a;
    let rgb = mix(vec3<f32>(1.0, 1.0, 1.0), s.rgb * u.tint.rgb, cov);
    return vec4<f32>(rgb, cov);
}

@fragment
fn fs_mipmap(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_sampler, in.uv);
}

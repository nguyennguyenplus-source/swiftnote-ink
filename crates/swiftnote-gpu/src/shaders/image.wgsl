struct ImageUniforms {
    view_bounds_pt: vec4<f32>, // page viewport min_x, min_y, max_x, max_y
    img_bounds_pt: vec4<f32>,  // image rect min_x, min_y, max_x, max_y
    crop_uv: vec4<f32>,        // crop min_u, min_v, max_u, max_v
    extra: vec4<f32>,          // x = rotation_rad, y = opacity
}

@group(0) @binding(0)
var<uniform> u: ImageUniforms;

@group(0) @binding(1)
var img_tex: texture_2d<f32>;

@group(0) @binding(2)
var img_sampler: sampler;

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
    let c = corners[vid];
    let center = vec2<f32>(
        (u.img_bounds_pt.x + u.img_bounds_pt.z) * 0.5,
        (u.img_bounds_pt.y + u.img_bounds_pt.w) * 0.5,
    );
    let pt_unrot = vec2<f32>(
        mix(u.img_bounds_pt.x, u.img_bounds_pt.z, c.x),
        mix(u.img_bounds_pt.y, u.img_bounds_pt.w, c.y),
    );
    let rel = pt_unrot - center;
    let ang = u.extra.x;
    let ca = cos(ang);
    let sa = sin(ang);
    let pt = center + vec2<f32>(rel.x * ca - rel.y * sa, rel.x * sa + rel.y * ca);

    let nx = (pt.x - u.view_bounds_pt.x) / max(u.view_bounds_pt.z - u.view_bounds_pt.x, 1e-4);
    let ny = (pt.y - u.view_bounds_pt.y) / max(u.view_bounds_pt.w - u.view_bounds_pt.y, 1e-4);
    let ndc = vec2<f32>(nx * 2.0 - 1.0, 1.0 - ny * 2.0);

    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = vec2<f32>(
        mix(u.crop_uv.x, u.crop_uv.z, c.x),
        mix(u.crop_uv.y, u.crop_uv.w, c.y),
    );
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(img_tex, img_sampler, in.uv);
    let op = u.extra.y;
    return vec4<f32>(sampled.rgb * (sampled.a * op), sampled.a * op);
}

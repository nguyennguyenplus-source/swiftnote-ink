struct StrokeUniforms {
    bounds_pt: vec4<f32>, // min_x, min_y, max_x, max_y in page points
    color: vec4<f32>,     // r, g, b, a (straight [0..1])
    mode: vec4<f32>,      // x: 0=Pen/Calligraphy, 1=Pencil, 2=HighlighterMask (opaque alpha=1 inside mask)
}

@group(0) @binding(0)
var<uniform> u: StrokeUniforms;

struct VsIn {
    @location(0) pos_pt: vec2<f32>,
    @location(1) uv: vec2<f32>,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let nx = (in.pos_pt.x - u.bounds_pt.x) / max(u.bounds_pt.z - u.bounds_pt.x, 1e-4);
    let ny = (in.pos_pt.y - u.bounds_pt.y) / max(u.bounds_pt.w - u.bounds_pt.y, 1e-4);
    let ndc = vec2<f32>(nx * 2.0 - 1.0, 1.0 - ny * 2.0);

    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

fn paper_grain(frag: vec2<f32>) -> f32 {
    let cell = floor(frag);
    let h = fract(sin(dot(cell, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let tooth = 0.5 + 0.5 * sin(frag.x * 0.75 + frag.y * 0.45);
    return mix(0.58, 0.98, 0.65 * h + 0.35 * tooth);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let mode = i32(u.mode.x + 0.5);
    if (mode == 2) {
        // Highlighter single-stroke coverage mask
        return vec4<f32>(u.color.rgb, 1.0);
    }
    var alpha = u.color.a;
    if (mode == 1) {
        alpha = alpha * paper_grain(in.pos.xy);
    }
    return vec4<f32>(u.color.rgb * alpha, alpha);
}

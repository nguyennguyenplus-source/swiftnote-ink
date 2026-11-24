struct Uniforms {
    bounds_pt: vec4<f32>, // min_x, min_y, max_x, max_y in page points
    params: vec4<f32>,    // template_id (0..5), spacing_pt, polarity (0=Light, 1=Blackboard), unused
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) page_pt: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Full-tile quad (2 triangles via 6 vertices)
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[vid];
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let px = mix(u.bounds_pt.x, u.bounds_pt.z, uv.x);
    let py = mix(u.bounds_pt.y, u.bounds_pt.w, uv.y);

    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.page_pt = vec2<f32>(px, py);
    return out;
}

fn srgb_to_linear_chan(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_to_linear_chan(c.r),
        srgb_to_linear_chan(c.g),
        srgb_to_linear_chan(c.b),
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let is_black = u.params.z > 0.5;
    let bg = srgb_to_linear(select(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(0.086, 0.090, 0.106), is_black));
    let line_col = srgb_to_linear(select(vec3<f32>(0.835, 0.855, 0.890), vec3<f32>(0.180, 0.196, 0.231), is_black));
    let accent_col = srgb_to_linear(select(vec3<f32>(0.74, 0.81, 0.90), vec3<f32>(0.24, 0.27, 0.34), is_black));

    let tid = i32(u.params.x + 0.5);
    let spacing = max(u.params.y, 4.0);
    let p = in.page_pt;
    let fw = max(fwidth(p), vec2<f32>(0.25, 0.25));

    var coverage = 0.0;
    var is_major = false;

    if (tid == 1) {
        // Ruled lines (vertical margin + horizontal rules)
        let dy = abs(fract(p.y / spacing + 0.5) - 0.5) * spacing;
        let line_a = 1.0 - smoothstep(0.0, fw.y * 1.15, dy);
        coverage = select(0.0, line_a, p.y >= spacing * 2.0);
        let margin_dx = abs(p.x - 72.0);
        if (margin_dx < fw.x * 1.2) {
            coverage = max(coverage, 1.0 - smoothstep(0.0, fw.x * 1.2, margin_dx));
            is_major = true;
        }
    } else if (tid == 2 || tid == 3) {
        // Math / Graph grid
        let dxy = abs(fract(p / spacing + 0.5) - 0.5) * spacing;
        let ax = 1.0 - smoothstep(0.0, fw.x * 1.15, dxy.x);
        let ay = 1.0 - smoothstep(0.0, fw.y * 1.15, dxy.y);
        coverage = max(ax, ay);
        if (tid == 3) {
            let major_sp = spacing * 5.0;
            let mdxy = abs(fract(p / major_sp + 0.5) - 0.5) * major_sp;
            let max_x = 1.0 - smoothstep(0.0, fw.x * 1.5, mdxy.x);
            let max_y = 1.0 - smoothstep(0.0, fw.y * 1.5, mdxy.y);
            if (max(max_x, max_y) > 0.1) {
                coverage = max(coverage, max(max_x, max_y));
                is_major = true;
            }
        }
    } else if (tid == 4) {
        // Dotted grid
        let cell = (fract(p / spacing + 0.5) - 0.5) * spacing;
        let d = length(cell);
        let dot_r = max(0.85, fw.x * 0.85);
        coverage = 1.0 - smoothstep(dot_r * 0.5, dot_r * 1.35, d);
    } else if (tid == 5) {
        // Isometric 30/60 degree triangle grid
        let s3 = 1.7320508;
        let v1 = abs(fract(p.x / spacing + 0.5) - 0.5) * spacing;
        let d2_proj = (0.5 * p.x + 0.5 * s3 * p.y) / spacing;
        let d3_proj = (0.5 * p.x - 0.5 * s3 * p.y) / spacing;
        let v2 = abs(fract(d2_proj + 0.5) - 0.5) * spacing;
        let v3 = abs(fract(d3_proj + 0.5) - 0.5) * spacing;
        let w = max(fw.x, fw.y) * 1.15;
        coverage = max(1.0 - smoothstep(0.0, w, v1), max(1.0 - smoothstep(0.0, w, v2), 1.0 - smoothstep(0.0, w, v3)));
    }

    let c = select(line_col, accent_col, is_major);
    let final_rgb = mix(bg, c, clamp(coverage, 0.0, 1.0));
    return vec4<f32>(final_rgb, 1.0);
}

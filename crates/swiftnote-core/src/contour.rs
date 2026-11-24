use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::brush::{BrushKind, BrushStyle};
use crate::stroke::StrokePoint;

const ARC_STEP_RAD: f64 = PI / 8.0;
const SINGLE_DOT_SEGMENTS: usize = 16;
const MIN_RADIUS_PT: f64 = 0.05;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourVertex {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContourMesh {
    pub vertices: Vec<ContourVertex>,
    pub indices: Vec<u32>,
}

impl ContourMesh {
    pub fn with_capacity(vertices: usize, indices: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(vertices),
            indices: Vec::with_capacity(indices),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }
}

#[derive(Debug, Clone, Copy)]
struct SegAngles {
    theta: f64,
    ang_l: f64,
    ang_r: f64,
}

/// Analytical variable-width contour generator sharing 100% identical geometry between wet & dry ink.
#[derive(Debug, Default)]
pub struct StrokeTessellator {
    clean_points: Vec<StrokePoint>,
    last_input_count: usize,
    emitted_segments: usize,
    dot_only: bool,
    end_cap_vert_start: usize,
    end_cap_idx_start: usize,
    last_seg_angles: Option<SegAngles>,
    last_style: Option<BrushStyle>,
}

impl StrokeTessellator {
    pub fn new() -> Self {
        Self {
            clean_points: Vec::with_capacity(256),
            last_input_count: 0,
            emitted_segments: 0,
            dot_only: false,
            end_cap_vert_start: 0,
            end_cap_idx_start: 0,
            last_seg_angles: None,
            last_style: None,
        }
    }

    pub fn reset(&mut self) {
        self.clean_points.clear();
        self.last_input_count = 0;
        self.emitted_segments = 0;
        self.dot_only = false;
        self.end_cap_vert_start = 0;
        self.end_cap_idx_start = 0;
        self.last_seg_angles = None;
        self.last_style = None;
    }

    pub fn tessellate_stroke(
        &mut self,
        points: &[StrokePoint],
        style: BrushStyle,
        out: &mut ContourMesh,
    ) {
        self.reset();
        self.tessellate_incremental(points, style, out);
    }

    pub fn tessellate_incremental(
        &mut self,
        points: &[StrokePoint],
        style: BrushStyle,
        out: &mut ContourMesh,
    ) {
        if points.is_empty() {
            self.reset();
            out.clear();
            return;
        }

        let first_width_changed = !self.clean_points.is_empty()
            && (points[0].width - self.clean_points[0].width).abs() > 1e-6;
        let need_rebuild = self.last_style != Some(style)
            || points.len() < self.last_input_count
            || self.last_input_count == 0
            || first_width_changed;

        if need_rebuild {
            self.clean_points.clear();
            self.emitted_segments = 0;
            self.dot_only = false;
            self.last_seg_angles = None;
            self.last_style = Some(style);
            out.clear();
            for &p in points {
                self.push_clean_point(p);
            }
        } else {
            for &p in &points[self.last_input_count..] {
                self.push_clean_point(p);
            }
        }
        self.last_input_count = points.len();

        let n = self.clean_points.len();
        if n == 0 {
            out.clear();
            return;
        }

        if n == 1 {
            if !self.dot_only {
                out.clear();
                let r = effective_radius_pt(self.clean_points[0], style, 0.0);
                emit_circle_fan(out, self.clean_points[0], r);
                self.dot_only = true;
            }
            return;
        }

        if self.dot_only {
            out.clear();
            self.dot_only = false;
            self.emitted_segments = 0;
            self.last_seg_angles = None;
        } else if self.emitted_segments > 0 {
            out.vertices.truncate(self.end_cap_vert_start);
            out.indices.truncate(self.end_cap_idx_start);
        }

        let round_caps = !matches!(style.kind, BrushKind::HighlighterSquare);

        while self.emitted_segments < n - 1 {
            let s_idx = self.emitted_segments;
            let p_a = self.clean_points[s_idx];
            let p_b = self.clean_points[s_idx + 1];
            let dx = p_b.x - p_a.x;
            let dy = p_b.y - p_a.y;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-5);
            let theta = dy.atan2(dx);

            let ra = effective_radius_pt(p_a, style, theta);
            let rb = effective_radius_pt(p_b, style, theta);
            let sin_alpha = ((ra - rb) / dist).clamp(-0.995, 0.995);
            let alpha = sin_alpha.asin();
            let seg_angles = SegAngles {
                theta,
                ang_l: theta + FRAC_PI_2 - alpha,
                ang_r: theta - FRAC_PI_2 + alpha,
            };

            if s_idx == 0 {
                if round_caps {
                    emit_start_cap(out, p_a, ra, seg_angles);
                }
            } else if let Some(prev_ang) = self.last_seg_angles {
                emit_joint(out, p_a, ra, prev_ang, seg_angles, round_caps);
            }

            emit_segment_quad(out, p_a, ra, p_b, rb, seg_angles);
            self.last_seg_angles = Some(seg_angles);
            self.emitted_segments += 1;
        }

        self.end_cap_vert_start = out.vertices.len();
        self.end_cap_idx_start = out.indices.len();
        if round_caps {
            if let Some(last_ang) = self.last_seg_angles {
                let last_pt = self.clean_points[n - 1];
                let r_last = effective_radius_pt(last_pt, style, last_ang.theta);
                emit_end_cap(out, last_pt, r_last, last_ang);
            }
        }
    }

    fn push_clean_point(&mut self, pt: StrokePoint) {
        if !pt.x.is_finite() || !pt.y.is_finite() || !pt.width.is_finite() {
            return;
        }
        if let Some(prev) = self.clean_points.last() {
            if prev.distance_to(pt) <= 1e-4 {
                return;
            }
        }
        self.clean_points.push(pt);
    }
}

#[inline]
fn effective_radius_pt(pt: StrokePoint, style: BrushStyle, theta: f64) -> f64 {
    let base_r = (f64::from(pt.width) * 0.5).max(MIN_RADIUS_PT);
    match style.kind {
        BrushKind::Calligraphy { nib_angle_deg } => {
            let phi = f64::from(nib_angle_deg).to_radians();
            let delta = theta - phi;
            let a = base_r * 1.25;
            let b = (base_r * 0.30).max(MIN_RADIUS_PT);
            ((a * delta.sin()).powi(2) + (b * delta.cos()).powi(2)).sqrt()
        }
        BrushKind::Pen
        | BrushKind::Pencil
        | BrushKind::HighlighterRound
        | BrushKind::HighlighterSquare => base_r,
    }
}

#[inline]
fn push_vert(out: &mut ContourMesh, x: f64, y: f64, u: f32, v: f32) -> u32 {
    let idx = out.vertices.len() as u32;
    out.vertices.push(ContourVertex {
        x: x as f32,
        y: y as f32,
        u,
        v,
    });
    idx
}

#[inline]
fn push_ccw_tri(out: &mut ContourMesh, i0: u32, i1: u32, i2: u32) {
    let a = out.vertices[i0 as usize];
    let b = out.vertices[i1 as usize];
    let c = out.vertices[i2 as usize];
    let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if cross >= 0.0 {
        out.indices.extend_from_slice(&[i0, i1, i2]);
    } else {
        out.indices.extend_from_slice(&[i0, i2, i1]);
    }
}

fn emit_circle_fan(out: &mut ContourMesh, center: StrokePoint, r: f64) {
    let c_idx = push_vert(out, center.x, center.y, 0.5, 0.5);
    let mut ring_indices = [0u32; SINGLE_DOT_SEGMENTS + 1];
    for (i, slot) in ring_indices.iter_mut().enumerate() {
        let ang = (i as f64 / SINGLE_DOT_SEGMENTS as f64) * TAU;
        let x = center.x + r * ang.cos();
        let y = center.y + r * ang.sin();
        *slot = push_vert(out, x, y, 1.0, 0.5);
    }
    for i in 0..SINGLE_DOT_SEGMENTS {
        push_ccw_tri(out, c_idx, ring_indices[i], ring_indices[i + 1]);
    }
}

fn emit_arc_fan(out: &mut ContourMesh, cx: f64, cy: f64, r: f64, a_start: f64, sweep: f64) {
    if sweep.abs() <= 1e-5 {
        return;
    }
    let steps = (sweep.abs() / ARC_STEP_RAD).ceil().max(1.0) as usize;
    let c_idx = push_vert(out, cx, cy, 0.5, 0.5);
    let mut prev_idx = push_vert(
        out,
        cx + r * a_start.cos(),
        cy + r * a_start.sin(),
        0.0,
        0.5,
    );
    for s in 1..=steps {
        let ang = a_start + sweep * (s as f64 / steps as f64);
        let next_idx = push_vert(out, cx + r * ang.cos(), cy + r * ang.sin(), 0.0, 0.5);
        push_ccw_tri(out, c_idx, prev_idx, next_idx);
        prev_idx = next_idx;
    }
}

fn emit_start_cap(out: &mut ContourMesh, p: StrokePoint, r: f64, seg: SegAngles) {
    let sweep = wrap_positive_2pi(seg.ang_r - seg.ang_l);
    emit_arc_fan(out, p.x, p.y, r, seg.ang_l, sweep);
}

fn emit_end_cap(out: &mut ContourMesh, p: StrokePoint, r: f64, seg: SegAngles) {
    let sweep = wrap_positive_2pi(seg.ang_l - seg.ang_r);
    emit_arc_fan(out, p.x, p.y, r, seg.ang_r, sweep);
}

fn emit_joint(
    out: &mut ContourMesh,
    p: StrokePoint,
    r: f64,
    prev: SegAngles,
    next: SegAngles,
    round_caps: bool,
) {
    let turn = wrap_signed_pi(next.theta - prev.theta);
    if turn.abs() <= 1e-4 {
        return;
    }
    let (a_start, sweep) = if turn > 0.0 {
        let sw = wrap_signed_pi(next.ang_r - prev.ang_r);
        (prev.ang_r, sw)
    } else {
        let sw = wrap_signed_pi(next.ang_l - prev.ang_l);
        (prev.ang_l, sw)
    };
    if sweep.abs() <= 1e-4 {
        return;
    }
    if round_caps {
        emit_arc_fan(out, p.x, p.y, r, a_start, sweep);
    } else {
        // Bevel triangle for square highlighter
        let c_idx = push_vert(out, p.x, p.y, 0.5, 0.5);
        let i1 = push_vert(
            out,
            p.x + r * a_start.cos(),
            p.y + r * a_start.sin(),
            0.0,
            0.5,
        );
        let end_ang = a_start + sweep;
        let i2 = push_vert(
            out,
            p.x + r * end_ang.cos(),
            p.y + r * end_ang.sin(),
            0.0,
            0.5,
        );
        push_ccw_tri(out, c_idx, i1, i2);
    }
}

fn emit_segment_quad(
    out: &mut ContourMesh,
    pa: StrokePoint,
    ra: f64,
    pb: StrokePoint,
    rb: f64,
    seg: SegAngles,
) {
    let (cl, sl) = (seg.ang_l.cos(), seg.ang_l.sin());
    let (cr, sr) = (seg.ang_r.cos(), seg.ang_r.sin());

    let la = push_vert(out, pa.x + ra * cl, pa.y + ra * sl, 0.0, 0.0);
    let ra_v = push_vert(out, pa.x + ra * cr, pa.y + ra * sr, 1.0, 0.0);
    let rb_v = push_vert(out, pb.x + rb * cr, pb.y + rb * sr, 1.0, 1.0);
    let lb = push_vert(out, pb.x + rb * cl, pb.y + rb * sl, 0.0, 1.0);

    push_ccw_tri(out, la, ra_v, rb_v);
    push_ccw_tri(out, la, rb_v, lb);
}

#[inline]
fn wrap_positive_2pi(mut ang: f64) -> f64 {
    ang = ang.rem_euclid(TAU);
    if ang < 1e-6 {
        0.0
    } else {
        ang
    }
}

#[inline]
fn wrap_signed_pi(ang: f64) -> f64 {
    let mut a = (ang + PI).rem_euclid(TAU) - PI;
    if a <= -PI {
        a += TAU;
    }
    a
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::brush::{BrushKind, Color};

    fn sample_style(kind: BrushKind) -> BrushStyle {
        BrushStyle {
            kind,
            color: Color::BLACK,
            width_pt: 4.0,
            pressure_sensitive: true,
        }
    }

    #[test]
    fn contour_single_dot_and_coincident_points_have_no_nan() {
        let mut tess = StrokeTessellator::new();
        let mut mesh = ContourMesh::default();
        let style = sample_style(BrushKind::Pen);

        tess.tessellate_stroke(
            &[StrokePoint::new(10.0, 20.0, 3.0, 0.0, 0.0)],
            style,
            &mut mesh,
        );
        assert!(mesh.vertices.len() >= 12);
        assert!(!mesh.indices.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.x.is_finite() && v.y.is_finite()));

        tess.tessellate_stroke(
            &[
                StrokePoint::new(10.0, 20.0, 3.0, 0.0, 0.0),
                StrokePoint::new(10.0, 20.0, 3.0, 0.0, 0.0),
            ],
            style,
            &mut mesh,
        );
        assert!(mesh.vertices.len() >= 12);
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.x.is_finite() && v.y.is_finite()));
    }

    #[test]
    fn contour_incremental_matches_committed_stroke_bit_for_bit() {
        let points = vec![
            StrokePoint::new(0.0, 0.0, 1.5, 0.0, 0.0),
            StrokePoint::new(12.0, 3.0, 2.8, 0.1, 0.0),
            StrokePoint::new(26.0, 18.0, 4.2, 0.1, 0.1),
            StrokePoint::new(14.0, 34.0, 3.0, 0.0, 0.2),
            StrokePoint::new(40.0, 42.0, 2.0, 0.0, 0.0),
        ];

        for kind in [
            BrushKind::Pen,
            BrushKind::Pencil,
            BrushKind::Calligraphy {
                nib_angle_deg: 45.0,
            },
            BrushKind::HighlighterRound,
            BrushKind::HighlighterSquare,
        ] {
            let style = sample_style(kind);
            let mut inc_tess = StrokeTessellator::new();
            let mut wet_mesh = ContourMesh::default();
            for end in 1..=points.len() {
                inc_tess.tessellate_incremental(&points[..end], style, &mut wet_mesh);
            }

            let mut dry_tess = StrokeTessellator::new();
            let mut dry_mesh = ContourMesh::default();
            dry_tess.tessellate_stroke(&points, style, &mut dry_mesh);

            assert_eq!(
                wet_mesh.vertices, dry_mesh.vertices,
                "vertex mismatch for {kind:?}"
            );
            assert_eq!(
                wet_mesh.indices, dry_mesh.indices,
                "index mismatch for {kind:?}"
            );
        }
    }

    #[test]
    fn contour_self_crossing_stroke_has_consistent_ccw_winding() {
        let points = vec![
            StrokePoint::new(0.0, 0.0, 3.0, 0.0, 0.0),
            StrokePoint::new(30.0, 0.0, 3.5, 0.0, 0.0),
            StrokePoint::new(30.0, 30.0, 4.0, 0.0, 0.0),
            StrokePoint::new(0.0, 15.0, 3.2, 0.0, 0.0),
            StrokePoint::new(40.0, 15.0, 2.5, 0.0, 0.0),
        ];
        for kind in [
            BrushKind::Pen,
            BrushKind::Pencil,
            BrushKind::Calligraphy {
                nib_angle_deg: 45.0,
            },
            BrushKind::HighlighterRound,
            BrushKind::HighlighterSquare,
        ] {
            let mut tess = StrokeTessellator::new();
            let mut mesh = ContourMesh::default();
            tess.tessellate_stroke(&points, sample_style(kind), &mut mesh);
            assert!(!mesh.indices.is_empty());
            assert_eq!(mesh.indices.len() % 3, 0);
            for tri in mesh.indices.as_chunks::<3>().0 {
                let a = mesh.vertices[tri[0] as usize];
                let b = mesh.vertices[tri[1] as usize];
                let c = mesh.vertices[tri[2] as usize];
                let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
                assert!(
                    cross >= -1e-4,
                    "inverted winding triangle in {kind:?}: cross={cross}"
                );
            }
        }
    }
}

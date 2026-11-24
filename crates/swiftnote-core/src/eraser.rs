use serde::{Deserialize, Serialize};

use crate::coords::{PagePoint, Rect};
use crate::document::Page;
use crate::stroke::{Stroke, StrokeId, StrokePoint};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EraserSize {
    Small,
    Medium,
    Large,
}

impl EraserSize {
    #[inline]
    pub const fn radius_pt(self) -> f64 {
        match self {
            Self::Small => 6.0,
            Self::Medium => 14.0,
            Self::Large => 28.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EraserMode {
    #[default]
    Stroke,
    Segment(EraserSize),
}

impl EraserMode {
    #[inline]
    pub const fn radius_pt(self) -> f64 {
        match self {
            Self::Stroke => 8.0,
            Self::Segment(sz) => sz.radius_pt(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EraseResult {
    pub removed: Vec<Stroke>,
    pub added: Vec<Stroke>,
    pub dirty_rect: Rect,
}

pub fn erase_at(
    page: &mut Page,
    center: PagePoint,
    prev_center: Option<PagePoint>,
    mode: EraserMode,
    alloc_id: &mut impl FnMut() -> StrokeId,
) -> EraseResult {
    let eraser_r = mode.radius_pt();
    let prev = prev_center.unwrap_or(center);
    let query_rect = Rect::from_point(prev.x, prev.y)
        .union(Rect::from_point(center.x, center.y))
        .expand(eraser_r);

    let candidate_ids = page.index.query_rect(query_rect);
    if candidate_ids.is_empty() {
        return EraseResult::default();
    }

    let mut result = EraseResult {
        removed: Vec::new(),
        added: Vec::new(),
        dirty_rect: Rect::EMPTY,
    };

    for id in candidate_ids {
        let Some(pos) = page.strokes.binary_search_by_key(&id, |s| s.id).ok() else {
            continue;
        };
        let stroke = &page.strokes[pos];
        let style_margin = stroke.style.max_half_width();
        let hit = match mode {
            EraserMode::Stroke => stroke_hits_capsule(stroke, prev, center, eraser_r, style_margin),
            EraserMode::Segment(_) => {
                stroke_hits_capsule(stroke, prev, center, eraser_r, style_margin)
            }
        };
        if !hit {
            continue;
        }

        let Some(removed_stroke) = page.remove_stroke_by_id(id) else {
            continue;
        };
        result.dirty_rect = result.dirty_rect.union(removed_stroke.bounds);

        if let EraserMode::Segment(_) = mode {
            let split_pieces =
                split_stroke_by_capsule(&removed_stroke, prev, center, eraser_r, alloc_id);
            for piece in split_pieces {
                result.dirty_rect = result.dirty_rect.union(piece.bounds);
                result.added.push(piece.clone());
                page.insert_stroke_sorted(piece);
            }
        }
        result.removed.push(removed_stroke);
    }

    result
}

fn stroke_hits_capsule(
    stroke: &Stroke,
    cap_a: PagePoint,
    cap_b: PagePoint,
    eraser_r: f64,
    style_margin: f64,
) -> bool {
    if stroke.points.is_empty() {
        return false;
    }
    if stroke.points.len() == 1 {
        let p = stroke.points[0];
        let hw = (f64::from(p.width) * 0.5).max(style_margin * 0.5);
        let d = point_to_segment_distance(PagePoint::new(p.x, p.y), cap_a, cap_b);
        return d <= eraser_r + hw;
    }

    for seg in stroke.points.windows(2) {
        let a = seg[0];
        let b = seg[1];
        let hw = f64::from(a.width.max(b.width)) * 0.5;
        let threshold = eraser_r + hw.max(style_margin * 0.5);
        if segment_to_segment_distance(
            PagePoint::new(a.x, a.y),
            PagePoint::new(b.x, b.y),
            cap_a,
            cap_b,
        ) <= threshold
        {
            return true;
        }
    }
    false
}

fn split_stroke_by_capsule(
    stroke: &Stroke,
    cap_a: PagePoint,
    cap_b: PagePoint,
    eraser_r: f64,
    alloc_id: &mut impl FnMut() -> StrokeId,
) -> Vec<Stroke> {
    if stroke.points.is_empty() {
        return Vec::new();
    }
    if stroke.points.len() == 1 {
        return Vec::new();
    }

    let style_half = stroke.style.max_half_width();
    let mut out = Vec::new();
    let mut current: Vec<StrokePoint> = Vec::new();

    for seg in stroke.points.windows(2) {
        let p0 = seg[0];
        let p1 = seg[1];
        let seg_hw = f64::from(p0.width.max(p1.width)) * 0.5;
        let threshold = eraser_r + seg_hw.max(style_half * 0.5);
        let a = PagePoint::new(p0.x, p0.y);
        let b = PagePoint::new(p1.x, p1.y);

        if segment_to_segment_distance(a, b, cap_a, cap_b) > threshold {
            if current.is_empty() {
                current.push(p0);
            }
            current.push(p1);
            continue;
        }

        // Subdivide hit segment finely so partial middle hits cleanly carve an opening.
        let steps = ((a.distance(b) / 2.0).ceil() as usize).clamp(4, 32);
        for s in 0..steps {
            let t0 = s as f64 / steps as f64;
            let t1 = (s + 1) as f64 / steps as f64;
            let sa = lerp_point(p0, p1, t0);
            let sb = lerp_point(p0, p1, t1);
            let d = segment_to_segment_distance(
                PagePoint::new(sa.x, sa.y),
                PagePoint::new(sb.x, sb.y),
                cap_a,
                cap_b,
            );
            let sub_hw = f64::from(sa.width.max(sb.width)) * 0.5;
            if d > eraser_r + sub_hw.max(style_half * 0.5) {
                if current.is_empty() {
                    current.push(sa);
                }
                current.push(sb);
            } else if current.len() >= 2 {
                out.push(Stroke::from_points(
                    alloc_id(),
                    stroke.style,
                    std::mem::take(&mut current),
                ));
            } else {
                current.clear();
            }
        }
    }

    if current.len() >= 2 {
        out.push(Stroke::from_points(alloc_id(), stroke.style, current));
    }

    out
}

#[inline]
fn lerp_point(a: StrokePoint, b: StrokePoint, t: f64) -> StrokePoint {
    let tf = t as f32;
    StrokePoint::new(
        a.x + (b.x - a.x) * t,
        a.y + (b.y - a.y) * t,
        a.width + (b.width - a.width) * tf,
        a.tilt_x + (b.tilt_x - a.tilt_x) * tf,
        a.tilt_y + (b.tilt_y - a.tilt_y) * tf,
    )
}

fn point_to_segment_distance(p: PagePoint, a: PagePoint, b: PagePoint) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= 1e-12 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
    let proj = PagePoint::new(a.x + t * dx, a.y + t * dy);
    p.distance(proj)
}

fn segment_to_segment_distance(a0: PagePoint, a1: PagePoint, b0: PagePoint, b1: PagePoint) -> f64 {
    if segments_intersect(a0, a1, b0, b1) {
        return 0.0;
    }
    point_to_segment_distance(a0, b0, b1)
        .min(point_to_segment_distance(a1, b0, b1))
        .min(point_to_segment_distance(b0, a0, a1))
        .min(point_to_segment_distance(b1, a0, a1))
}

fn segments_intersect(a0: PagePoint, a1: PagePoint, b0: PagePoint, b1: PagePoint) -> bool {
    let orient = |p: PagePoint, q: PagePoint, r: PagePoint| -> f64 {
        (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
    };
    let o1 = orient(a0, a1, b0);
    let o2 = orient(a0, a1, b1);
    let o3 = orient(b0, b1, a0);
    let o4 = orient(b0, b1, a1);
    (o1 * o2 < 0.0) && (o3 * o4 < 0.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::brush::{BrushKind, BrushStyle, Color};
    use crate::document::PageId;
    use crate::paper::PageBackground;

    #[test]
    fn segment_eraser_accounts_for_stroke_half_width_and_splits_cleanly() {
        let wide_style = BrushStyle {
            kind: BrushKind::Pen,
            color: Color::BLUE,
            width_pt: 16.0, // half width = 8 pt (or up to 10.8 pt with pressure)
            pressure_sensitive: false,
        };
        let stroke = Stroke::from_points(
            StrokeId(10),
            wide_style,
            vec![
                StrokePoint::new(0.0, 50.0, 16.0, 0.0, 0.0),
                StrokePoint::new(120.0, 50.0, 16.0, 0.0, 0.0),
            ],
        );
        let mut page = Page::new(PageId(1), 595.28, 841.89, PageBackground::default());
        page.insert_stroke_sorted(stroke);

        let mut next_id = 100u64;
        let mut alloc = || {
            let id = StrokeId(next_id);
            next_id += 1;
            id
        };

        // Erase at y = 39.0 (11 pt from centerline: within 6pt Small radius + 8pt stroke half-width = 14pt)
        let res = erase_at(
            &mut page,
            PagePoint::new(60.0, 39.0),
            None,
            EraserMode::Segment(EraserSize::Small),
            &mut alloc,
        );
        assert_eq!(res.removed.len(), 1);
        assert_eq!(
            res.added.len(),
            2,
            "middle graze should split into 2 pieces"
        );
        assert!(res.added.iter().all(|s| s.style == wide_style));
        assert!(res.added[0].bounds.max_x < 60.0);
        assert!(res.added[1].bounds.min_x > 60.0);
    }
}

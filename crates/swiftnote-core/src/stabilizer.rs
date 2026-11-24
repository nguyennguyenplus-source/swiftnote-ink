use std::collections::VecDeque;

use crate::brush::BrushStyle;
use crate::settings::StabilizerPreset;
use crate::stroke::StrokePoint;

/// Minimum motion distance (pt) before emitting another vertex (Xournal++ PIXEL_MOTION_THRESHOLD).
pub const PIXEL_MOTION_THRESHOLD: f64 = 0.4;

/// Maximum width change (pt) per segment before decomposing into smooth sub-steps.
pub const MAX_WIDTH_VARIATION: f32 = 0.6;

/// Maximum logical jump accepted on proximity enter/leave before rejecting hardware glitch spikes.
pub const MAX_PROXIMITY_JUMP_PX: f64 = 100.0;

/// Maximum single-packet motion jump (pt) before rejecting corrupted virtual-tablet glitches.
const MAX_MOTION_JUMP_PT: f64 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawInputSample {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub timestamp_ms: u64,
}

impl RawInputSample {
    pub const fn new(
        x: f64,
        y: f64,
        pressure: f64,
        tilt_x: f32,
        tilt_y: f32,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            x,
            y,
            pressure,
            tilt_x,
            tilt_y,
            timestamp_ms,
        }
    }
}

/// Filters corrupted hardware packets, zero-pressure lift artifacts, and proximity teleports.
#[derive(Debug, Clone, Default)]
pub struct InputGuard {
    last_raw: Option<(f64, f64)>,
}

impl InputGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.last_raw = None;
    }

    pub fn filter_down(&mut self, sample: RawInputSample) -> Option<RawInputSample> {
        if !sample.x.is_finite() || !sample.y.is_finite() {
            return None;
        }
        let pressure = if sample.pressure.is_finite() && sample.pressure > 0.0 {
            sample.pressure.clamp(0.05, 1.0)
        } else {
            0.5
        };
        let tilt_x = if sample.tilt_x.is_finite() {
            sample.tilt_x.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let tilt_y = if sample.tilt_y.is_finite() {
            sample.tilt_y.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.last_raw = Some((sample.x, sample.y));
        Some(RawInputSample {
            x: sample.x,
            y: sample.y,
            pressure,
            tilt_x,
            tilt_y,
            timestamp_ms: sample.timestamp_ms,
        })
    }

    pub fn filter_motion(&mut self, sample: RawInputSample) -> Option<RawInputSample> {
        if !sample.x.is_finite()
            || !sample.y.is_finite()
            || !sample.pressure.is_finite()
            || sample.pressure <= 0.0
        {
            return None;
        }
        if let Some((lx, ly)) = self.last_raw {
            let dist = ((sample.x - lx).powi(2) + (sample.y - ly).powi(2)).sqrt();
            if dist > MAX_MOTION_JUMP_PT {
                return None;
            }
        }
        let pressure = sample.pressure.clamp(0.05, 1.0);
        let tilt_x = if sample.tilt_x.is_finite() {
            sample.tilt_x.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let tilt_y = if sample.tilt_y.is_finite() {
            sample.tilt_y.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.last_raw = Some((sample.x, sample.y));
        Some(RawInputSample {
            x: sample.x,
            y: sample.y,
            pressure,
            tilt_x,
            tilt_y,
            timestamp_ms: sample.timestamp_ms,
        })
    }

    pub fn filter_enter_leave(&mut self, sample: RawInputSample) -> bool {
        if !sample.x.is_finite() || !sample.y.is_finite() {
            return false;
        }
        if let Some((lx, ly)) = self.last_raw {
            if (sample.x - lx).abs() > MAX_PROXIMITY_JUMP_PX
                || (sample.y - ly).abs() > MAX_PROXIMITY_JUMP_PX
            {
                return false;
            }
        }
        self.last_raw = Some((sample.x, sample.y));
        true
    }
}

#[derive(Debug, Clone, Copy)]
struct VelocitySample {
    x: f64,
    y: f64,
    pressure: f64,
    tilt_x: f32,
    tilt_y: f32,
    velocity: f64,
}

/// Xournal++-derived Cusp-Detecting Deadzone + Velocity-Integral Gaussian stabilizer.
#[derive(Debug, Clone)]
pub struct StrokeStabilizer {
    preset: StabilizerPreset,
    buffer: VecDeque<VelocitySample>,
    last_raw: Option<(f64, f64, u64, f64, f32, f32)>,
    last_emitted: Option<StrokePoint>,
    deadzone_anchor: Option<(f64, f64, f64, f32, f32)>,
    deadzone_prev_dir: Option<(f64, f64)>,
}

impl StrokeStabilizer {
    pub fn new(preset: StabilizerPreset) -> Self {
        Self {
            preset,
            buffer: VecDeque::with_capacity(32),
            last_raw: None,
            last_emitted: None,
            deadzone_anchor: None,
            deadzone_prev_dir: None,
        }
    }

    pub fn set_preset(&mut self, preset: StabilizerPreset) {
        self.preset = preset;
    }

    pub fn preset(&self) -> StabilizerPreset {
        self.preset
    }

    fn params(self_preset: StabilizerPreset) -> (f64, f64) {
        match self_preset {
            StabilizerPreset::Off => (0.0, 0.0),
            StabilizerPreset::Light => (1.2, 2.0 * 8.0 * 8.0),
            StabilizerPreset::Medium => (2.0, 2.0 * 16.0 * 16.0),
            StabilizerPreset::Strong => (3.0, 2.0 * 28.0 * 28.0),
        }
    }

    #[inline]
    fn compute_width(style: BrushStyle, pressure: f64) -> f32 {
        if style.pressure_sensitive {
            let factor = 0.35 + 0.95 * pressure.clamp(0.0, 1.0);
            (f64::from(style.width_pt) * factor) as f32
        } else {
            style.width_pt
        }
    }

    pub fn begin(&mut self, sample: RawInputSample, style: BrushStyle, out: &mut Vec<StrokePoint>) {
        self.buffer.clear();
        self.last_raw = Some((
            sample.x,
            sample.y,
            sample.timestamp_ms,
            sample.pressure,
            sample.tilt_x,
            sample.tilt_y,
        ));
        self.deadzone_anchor = Some((
            sample.x,
            sample.y,
            sample.pressure,
            sample.tilt_x,
            sample.tilt_y,
        ));
        self.deadzone_prev_dir = None;
        self.buffer.push_front(VelocitySample {
            x: sample.x,
            y: sample.y,
            pressure: sample.pressure,
            tilt_x: sample.tilt_x,
            tilt_y: sample.tilt_y,
            velocity: 0.0,
        });

        let pt = StrokePoint::new(
            sample.x,
            sample.y,
            Self::compute_width(style, sample.pressure),
            sample.tilt_x,
            sample.tilt_y,
        );
        out.push(pt);
        self.last_emitted = Some(pt);
    }

    pub fn push(&mut self, sample: RawInputSample, style: BrushStyle, out: &mut Vec<StrokePoint>) {
        let (deadzone_radius, two_sigma_sq) = Self::params(self.preset);
        if self.preset == StabilizerPreset::Off {
            let next = StrokePoint::new(
                sample.x,
                sample.y,
                Self::compute_width(style, sample.pressure),
                sample.tilt_x,
                sample.tilt_y,
            );
            self.emit_filtered(next, out);
            self.last_raw = Some((
                sample.x,
                sample.y,
                sample.timestamp_ms,
                sample.pressure,
                sample.tilt_x,
                sample.tilt_y,
            ));
            return;
        }

        // Stage 1: Deadzone Preprocessor with Cusp Detection (>90 deg direction change)
        if let Some((ax, ay, ap, atx, aty)) = self.deadzone_anchor {
            let dx = sample.x - ax;
            let dy = sample.y - ay;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < deadzone_radius {
                return;
            }
            let dir = (dx / dist, dy / dist);
            if let Some((px, py)) = self.deadzone_prev_dir {
                let dot = px * dir.0 + py * dir.1;
                if dot < 0.0 {
                    // Cusp corner: flush anchor verbatim and reset Gaussian velocity window.
                    let cusp_pt =
                        StrokePoint::new(ax, ay, Self::compute_width(style, ap), atx, aty);
                    self.emit_filtered(cusp_pt, out);
                    self.buffer.clear();
                    self.buffer.push_front(VelocitySample {
                        x: ax,
                        y: ay,
                        pressure: ap,
                        tilt_x: atx,
                        tilt_y: aty,
                        velocity: 0.0,
                    });
                }
            }
            self.deadzone_prev_dir = Some(dir);
            self.deadzone_anchor = Some((
                sample.x,
                sample.y,
                sample.pressure,
                sample.tilt_x,
                sample.tilt_y,
            ));
        } else {
            self.deadzone_anchor = Some((
                sample.x,
                sample.y,
                sample.pressure,
                sample.tilt_x,
                sample.tilt_y,
            ));
        }

        // Stage 2: Velocity-Integral Gaussian
        let vel = if let Some((lx, ly, lt, ..)) = self.last_raw {
            let dt = sample.timestamp_ms.saturating_sub(lt).max(1) as f64;
            let step_dist = ((sample.x - lx).powi(2) + (sample.y - ly).powi(2)).sqrt();
            step_dist / dt
        } else {
            0.0
        };
        self.last_raw = Some((
            sample.x,
            sample.y,
            sample.timestamp_ms,
            sample.pressure,
            sample.tilt_x,
            sample.tilt_y,
        ));

        if self.buffer.len() >= 32 {
            self.buffer.pop_back();
        }
        self.buffer.push_front(VelocitySample {
            x: sample.x,
            y: sample.y,
            pressure: sample.pressure,
            tilt_x: sample.tilt_x,
            tilt_y: sample.tilt_y,
            velocity: vel,
        });

        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_p = 0.0;
        let mut sum_tx = 0.0;
        let mut sum_ty = 0.0;
        let mut sum_w = 0.0;
        let mut sum_v = 0.0;
        let mut keep_len = 0usize;

        for s in &self.buffer {
            let w = (-sum_v * sum_v / two_sigma_sq).exp();
            if w < 0.01 {
                break;
            }
            keep_len += 1;
            sum_v += s.velocity;
            sum_x += w * s.x;
            sum_y += w * s.y;
            sum_p += w * s.pressure;
            sum_tx += w * f64::from(s.tilt_x);
            sum_ty += w * f64::from(s.tilt_y);
            sum_w += w;
        }
        self.buffer.truncate(keep_len.max(1));

        let next = if sum_w > 0.0 {
            StrokePoint::new(
                sum_x / sum_w,
                sum_y / sum_w,
                Self::compute_width(style, sum_p / sum_w),
                (sum_tx / sum_w) as f32,
                (sum_ty / sum_w) as f32,
            )
        } else {
            StrokePoint::new(
                sample.x,
                sample.y,
                Self::compute_width(style, sample.pressure),
                sample.tilt_x,
                sample.tilt_y,
            )
        };
        self.emit_filtered(next, out);
    }

    pub fn finish(
        &mut self,
        final_sample: RawInputSample,
        style: BrushStyle,
        out: &mut Vec<StrokePoint>,
    ) {
        let target_w = Self::compute_width(style, final_sample.pressure);
        let target = StrokePoint::new(
            final_sample.x,
            final_sample.y,
            target_w,
            final_sample.tilt_x,
            final_sample.tilt_y,
        );
        let Some(prev) = self.last_emitted else {
            out.push(target);
            self.last_emitted = Some(target);
            return;
        };

        let dist = prev.distance_to(target);
        if dist <= 1e-4 {
            return;
        }

        // Quadratic spline tail flush when lag > 2 * PIXEL_MOTION_THRESHOLD without modifying prior points.
        if self.preset != StabilizerPreset::Off && dist > PIXEL_MOTION_THRESHOLD * 2.0 {
            let ctrl = if let Some((ax, ay, ..)) = self.deadzone_anchor {
                (ax, ay)
            } else {
                ((prev.x + target.x) * 0.5, (prev.y + target.y) * 0.5)
            };
            let steps = 2usize;
            for step in 1..=steps {
                let t = step as f64 / steps as f64;
                let inv = 1.0 - t;
                let x = inv * inv * prev.x + 2.0 * inv * t * ctrl.0 + t * t * target.x;
                let y = inv * inv * prev.y + 2.0 * inv * t * ctrl.1 + t * t * target.y;
                let w = prev.width + (target.width - prev.width) * (t as f32);
                let tx = prev.tilt_x + (target.tilt_x - prev.tilt_x) * (t as f32);
                let ty = prev.tilt_y + (target.tilt_y - prev.tilt_y) * (t as f32);
                let mid = StrokePoint::new(x, y, w, tx, ty);
                if let Some(last) = self.last_emitted {
                    decompose_pressure_step(last, mid, out);
                    if out.last().copied() != Some(mid) && step == steps {
                        out.push(mid);
                    }
                    self.last_emitted = out.last().copied().or(Some(mid));
                }
            }
        } else {
            decompose_pressure_step(prev, target, out);
            if out.last().copied() != Some(target) {
                out.push(target);
            }
            self.last_emitted = out.last().copied();
        }
    }

    fn emit_filtered(&mut self, next: StrokePoint, out: &mut Vec<StrokePoint>) {
        let Some(prev) = self.last_emitted else {
            out.push(next);
            self.last_emitted = Some(next);
            return;
        };
        let dist = prev.distance_to(next);
        if dist < PIXEL_MOTION_THRESHOLD {
            // Xournal++ first-point pressure thickening when stylus settles before moving
            if out.len() == 1 && next.width > prev.width {
                if let Some(first) = out.first_mut() {
                    first.width = next.width;
                    self.last_emitted = Some(*first);
                }
            }
            return;
        }
        decompose_pressure_step(prev, next, out);
        self.last_emitted = out.last().copied();
    }
}

pub fn decompose_pressure_step(prev: StrokePoint, next: StrokePoint, out: &mut Vec<StrokePoint>) {
    let dw = (next.width - prev.width).abs();
    let dist = prev.distance_to(next);
    if dw <= MAX_WIDTH_VARIATION || dist < PIXEL_MOTION_THRESHOLD {
        out.push(next);
        return;
    }

    let max_by_dist = (dist / PIXEL_MOTION_THRESHOLD).floor().max(1.0);
    let steps = (f64::from(dw / MAX_WIDTH_VARIATION).ceil())
        .min(max_by_dist)
        .clamp(1.0, 16.0) as usize;

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let tf = t as f32;
        out.push(StrokePoint::new(
            prev.x + (next.x - prev.x) * t,
            prev.y + (next.y - prev.y) * t,
            prev.width + (next.width - prev.width) * tf,
            prev.tilt_x + (next.tilt_x - prev.tilt_x) * tf,
            prev.tilt_y + (next.tilt_y - prev.tilt_y) * tf,
        ));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::brush::default_pen_slots;

    #[test]
    fn input_guard_drops_nan_pressure_zero_and_teleport_spikes() {
        let mut guard = InputGuard::new();
        assert!(guard
            .filter_down(RawInputSample::new(f64::NAN, 10.0, 0.5, 0.0, 0.0, 0))
            .is_none());
        assert!(guard
            .filter_down(RawInputSample::new(10.0, f64::INFINITY, 0.5, 0.0, 0.0, 0))
            .is_none());

        let down = guard.filter_down(RawInputSample::new(50.0, 50.0, 0.4, 0.0, 0.0, 10));
        assert!(down.is_some());

        // Lift artifact pressure == 0.0 must be dropped
        assert!(guard
            .filter_motion(RawInputSample::new(55.0, 55.0, 0.0, 0.0, 0.0, 15))
            .is_none());

        // Enter/leave jump > 100 logical px must be discarded
        assert!(!guard.filter_enter_leave(RawInputSample::new(300.0, 50.0, 0.5, 0.0, 0.0, 20)));
        assert!(guard.filter_enter_leave(RawInputSample::new(70.0, 60.0, 0.5, 0.0, 0.0, 20)));
    }

    #[test]
    fn stabilizer_preserves_sharp_math_cusp_and_flushes_tail() {
        let style = default_pen_slots()[0];
        let mut stab = StrokeStabilizer::new(StabilizerPreset::Medium);
        let mut pts = Vec::with_capacity(64);

        // Draw a sharp '2' / '>' cusp corner at (100.0, 40.0): move right then sharply down-left
        stab.begin(
            RawInputSample::new(40.0, 40.0, 0.7, 0.0, 0.0, 0),
            style,
            &mut pts,
        );
        for i in 1u32..=6 {
            let x = 40.0 + f64::from(i) * 10.0;
            stab.push(
                RawInputSample::new(x, 40.0, 0.7, 0.0, 0.0, u64::from(i) * 5),
                style,
                &mut pts,
            );
        }
        let prefix_len_before_cusp = pts.len();
        // Sharp reversal (> 90 deg) from (100, 40) to (60, 80)
        for i in 1u32..=5 {
            let t = f64::from(i) / 5.0;
            let x = 100.0 - 40.0 * t;
            let y = 40.0 + 40.0 * t;
            stab.push(
                RawInputSample::new(x, y, 0.7, 0.0, 0.0, 35 + u64::from(i) * 5),
                style,
                &mut pts,
            );
        }

        // Prefix before cusp was NOT mutated when finishing
        let snapshot_before_finish = pts[..prefix_len_before_cusp].to_vec();
        let final_pt = RawInputSample::new(50.0, 90.0, 0.7, 0.0, 0.0, 70);
        stab.finish(final_pt, style, &mut pts);
        assert_eq!(
            &pts[..prefix_len_before_cusp],
            snapshot_before_finish.as_slice()
        );

        // Cusp apex stays crisp (some vertex within 3.5 pt of (100, 40))
        let min_dist_to_cusp = pts
            .iter()
            .map(|p| ((p.x - 100.0).powi(2) + (p.y - 40.0).powi(2)).sqrt())
            .fold(f64::INFINITY, f64::min);
        assert!(
            min_dist_to_cusp <= 3.5,
            "cusp rounded too aggressively: min_dist = {min_dist_to_cusp}"
        );

        // Final point reaches exact final pen location
        let last = pts.last().unwrap();
        assert!((last.x - 50.0).abs() < 1e-6 && (last.y - 90.0).abs() < 1e-6);
    }
}

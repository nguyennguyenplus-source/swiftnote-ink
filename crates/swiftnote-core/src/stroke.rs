use serde::{Deserialize, Serialize};

use crate::brush::BrushStyle;
use crate::coords::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StrokeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    pub width: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
}

impl StrokePoint {
    #[inline]
    pub const fn new(x: f64, y: f64, width: f32, tilt_x: f32, tilt_y: f32) -> Self {
        Self {
            x,
            y,
            width,
            tilt_x,
            tilt_y,
        }
    }

    #[inline]
    pub fn distance_to(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: StrokeId,
    pub style: BrushStyle,
    pub points: Vec<StrokePoint>,
    pub bounds: Rect,
}

impl Stroke {
    pub fn new(id: StrokeId, style: BrushStyle) -> Self {
        Self {
            id,
            style,
            points: Vec::with_capacity(64),
            bounds: Rect::EMPTY,
        }
    }

    pub fn from_points(id: StrokeId, style: BrushStyle, points: Vec<StrokePoint>) -> Self {
        let mut s = Self {
            id,
            style,
            points,
            bounds: Rect::EMPTY,
        };
        s.recompute_bounds();
        s
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn append_point(&mut self, pt: StrokePoint) {
        let half = f64::from(pt.width) * 0.5;
        let pt_rect = Rect::from_point(pt.x, pt.y).expand(half);
        self.bounds = self.bounds.union(pt_rect);
        self.points.push(pt);
    }

    pub fn recompute_bounds(&mut self) {
        let mut rect = Rect::EMPTY;
        for pt in &self.points {
            let half = f64::from(pt.width) * 0.5;
            rect = rect.union(Rect::from_point(pt.x, pt.y).expand(half));
        }
        self.bounds = rect;
    }
}

use serde::{Deserialize, Serialize};

/// Logical window pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

/// Physical framebuffer pixels (`ScreenPoint * display_scale`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PhysicalPoint {
    pub x: f64,
    pub y: f64,
}

/// Infinite vertical canvas space in typographic points (`72 pt = 1 inch`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CanvasPoint {
    pub x: f64,
    pub y: f64,
}

/// Page-local coordinates in typographic points (`72 pt = 1 inch`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PagePoint {
    pub x: f64,
    pub y: f64,
}

macro_rules! impl_point_methods {
    ($name:ident) => {
        impl $name {
            #[inline]
            pub const fn new(x: f64, y: f64) -> Self {
                Self { x, y }
            }

            #[inline]
            pub fn is_finite(self) -> bool {
                self.x.is_finite() && self.y.is_finite()
            }

            #[inline]
            pub fn distance_sq(self, other: Self) -> f64 {
                let dx = self.x - other.x;
                let dy = self.y - other.y;
                dx * dx + dy * dy
            }

            #[inline]
            pub fn distance(self, other: Self) -> f64 {
                self.distance_sq(other).sqrt()
            }

            #[inline]
            pub fn lerp(self, other: Self, t: f64) -> Self {
                Self {
                    x: self.x + (other.x - self.x) * t,
                    y: self.y + (other.y - self.y) * t,
                }
            }
        }
    };
}

impl_point_methods!(ScreenPoint);
impl_point_methods!(PhysicalPoint);
impl_point_methods!(CanvasPoint);
impl_point_methods!(PagePoint);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Default for Rect {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Rect {
    pub const EMPTY: Self = Self {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };

    #[inline]
    pub const fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    #[inline]
    pub fn from_point(x: f64, y: f64) -> Self {
        Self::new(x, y, x, y)
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }

    #[inline]
    pub fn width(self) -> f64 {
        (self.max_x - self.min_x).max(0.0)
    }

    #[inline]
    pub fn height(self) -> f64 {
        (self.max_y - self.min_y).max(0.0)
    }

    #[inline]
    pub fn include_point(&mut self, x: f64, y: f64) {
        if x < self.min_x {
            self.min_x = x;
        }
        if y < self.min_y {
            self.min_y = y;
        }
        if x > self.max_x {
            self.max_x = x;
        }
        if y > self.max_y {
            self.max_y = y;
        }
    }

    #[inline]
    pub fn expand(self, padding: f64) -> Self {
        if self.is_empty() {
            return self;
        }
        Self {
            min_x: self.min_x - padding,
            min_y: self.min_y - padding,
            max_x: self.max_x + padding,
            max_y: self.max_y + padding,
        }
    }

    #[inline]
    pub fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }

    #[inline]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    #[inline]
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewportTransform {
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
}

impl Default for ViewportTransform {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl ViewportTransform {
    pub const MIN_ZOOM: f64 = 0.1;
    pub const MAX_ZOOM: f64 = 8.0;

    #[inline]
    pub fn screen_to_canvas(self, screen: ScreenPoint) -> CanvasPoint {
        CanvasPoint::new(
            (screen.x - self.pan_x) / self.zoom,
            (screen.y - self.pan_y) / self.zoom,
        )
    }

    #[inline]
    pub fn canvas_to_screen(self, canvas: CanvasPoint) -> ScreenPoint {
        ScreenPoint::new(
            canvas.x * self.zoom + self.pan_x,
            canvas.y * self.zoom + self.pan_y,
        )
    }

    #[inline]
    pub fn set_zoom_at(&mut self, anchor: ScreenPoint, new_zoom: f64) {
        if !new_zoom.is_finite() || !anchor.is_finite() {
            return;
        }
        let clamped = new_zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let world = self.screen_to_canvas(anchor);
        self.zoom = clamped;
        self.pan_x = anchor.x - world.x * clamped;
        self.pan_y = anchor.y - world.y * clamped;
    }

    #[inline]
    pub fn pan_by(&mut self, dx_screen: f64, dy_screen: f64) {
        if dx_screen.is_finite() && dy_screen.is_finite() {
            self.pan_x += dx_screen;
            self.pan_y += dy_screen;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn zoom_clamps_to_bounds() {
        let mut vp = ViewportTransform::default();
        let anchor = ScreenPoint::new(400.0, 300.0);
        vp.set_zoom_at(anchor, 0.01);
        assert!((vp.zoom - ViewportTransform::MIN_ZOOM).abs() < 1e-12);
        vp.set_zoom_at(anchor, 42.0);
        assert!((vp.zoom - ViewportTransform::MAX_ZOOM).abs() < 1e-12);
    }

    proptest! {
        #[test]
        fn viewport_roundtrip_proptest(
            pan_x in -5000.0f64..5000.0,
            pan_y in -5000.0f64..5000.0,
            zoom in 0.1f64..8.0,
            sx in -2000.0f64..4000.0,
            sy in -2000.0f64..4000.0,
        ) {
            let vp = ViewportTransform { pan_x, pan_y, zoom };
            let screen = ScreenPoint::new(sx, sy);
            let canvas = vp.screen_to_canvas(screen);
            let back = vp.canvas_to_screen(canvas);
            prop_assert!((back.x - sx).abs() < 1e-6);
            prop_assert!((back.y - sy).abs() < 1e-6);
        }

        #[test]
        fn zoom_preserves_anchor_proptest(
            pan_x in -2000.0f64..2000.0,
            pan_y in -2000.0f64..2000.0,
            zoom in 0.2f64..4.0,
            new_zoom in 0.1f64..8.0,
            ax in 0.0f64..2560.0,
            ay in 0.0f64..1440.0,
        ) {
            let mut vp = ViewportTransform { pan_x, pan_y, zoom };
            let anchor = ScreenPoint::new(ax, ay);
            let world_before = vp.screen_to_canvas(anchor);
            vp.set_zoom_at(anchor, new_zoom);
            let world_after = vp.screen_to_canvas(anchor);
            prop_assert!((world_after.x - world_before.x).abs() < 1e-6);
            prop_assert!((world_after.y - world_before.y).abs() < 1e-6);
        }
    }
}

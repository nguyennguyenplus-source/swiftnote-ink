use serde::{Deserialize, Serialize};

use crate::paper::PaperPolarity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0x11, 0x11, 0x13);
    pub const PURE_BLACK: Self = Self::rgb(0x00, 0x00, 0x00);
    pub const WHITE: Self = Self::rgb(0xFF, 0xFF, 0xFF);
    pub const CHALK_WHITE: Self = Self::rgb(0xF4, 0xF4, 0xF6);
    pub const BLUE: Self = Self::rgb(0x00, 0x66, 0xFF);
    pub const RED: Self = Self::rgb(0xDC, 0x26, 0x26);
    pub const GREEN: Self = Self::rgb(0x16, 0xA3, 0x4A);
    pub const PENCIL_GRAPHITE: Self = Self::rgb(0x3F, 0x3F, 0x46);
    pub const HIGHLIGHTER_YELLOW: Self = Self::rgba(0xFA, 0xCC, 0x15, 115);

    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    pub fn is_dark(self) -> bool {
        let lum =
            0.2126 * f64::from(self.r) + 0.7152 * f64::from(self.g) + 0.0722 * f64::from(self.b);
        lum < 128.0
    }

    #[inline]
    pub fn is_neutral_monochrome(self) -> bool {
        let max_c = self.r.max(self.g).max(self.b);
        let min_c = self.r.min(self.g).min(self.b);
        let chroma = max_c - min_c;
        chroma <= 18 && (max_c <= 48 || min_c >= 207)
    }

    pub fn effective_for_paper(
        self,
        polarity: PaperPolarity,
        auto_invert_monochrome: bool,
    ) -> Self {
        if polarity == PaperPolarity::Blackboard
            && auto_invert_monochrome
            && self.is_neutral_monochrome()
        {
            return Self::rgba(255 - self.r, 255 - self.g, 255 - self.b, self.a);
        }
        self
    }
}

pub const DEFAULT_PALETTE: &[(&str, Color)] = &[
    ("Ink Black", Color::BLACK),
    ("Cobalt Blue", Color::BLUE),
    ("Crimson Red", Color::RED),
    ("Forest Green", Color::GREEN),
    ("Royal Purple", Color::rgb(0x7C, 0x3A, 0xED)),
    ("Amber Orange", Color::rgb(0xEA, 0x58, 0x0C)),
    ("Ocean Teal", Color::rgb(0x0D, 0x94, 0x88)),
    ("Sky Blue", Color::rgb(0x02, 0x84, 0xC7)),
    ("Yellow Highlighter", Color::HIGHLIGHTER_YELLOW),
    ("Pink Highlighter", Color::rgba(0xF4, 0x72, 0xB6, 115)),
    ("Green Highlighter", Color::rgba(0x4A, 0xDE, 0x80, 115)),
    ("Cyan Highlighter", Color::rgba(0x38, 0xBD, 0xF8, 115)),
    ("Graphite", Color::PENCIL_GRAPHITE),
    ("Slate Gray", Color::rgb(0x64, 0x74, 0x8B)),
    ("Chalk Yellow", Color::rgb(0xFD, 0xE0, 0x47)),
    ("Chalk White", Color::CHALK_WHITE),
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BrushKind {
    Pen,
    Pencil,
    Calligraphy { nib_angle_deg: f32 },
    HighlighterRound,
    HighlighterSquare,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BrushStyle {
    pub kind: BrushKind,
    pub color: Color,
    pub width_pt: f32,
    pub pressure_sensitive: bool,
}

impl BrushStyle {
    #[inline]
    pub fn is_highlighter(self) -> bool {
        matches!(
            self.kind,
            BrushKind::HighlighterRound | BrushKind::HighlighterSquare
        )
    }

    #[inline]
    pub fn max_half_width(self) -> f64 {
        let scale = if self.pressure_sensitive { 1.35 } else { 1.0 };
        f64::from(self.width_pt) * 0.5 * scale
    }
}

pub fn default_pen_slots() -> [BrushStyle; 5] {
    [
        BrushStyle {
            kind: BrushKind::Pen,
            color: Color::BLACK,
            width_pt: 2.2,
            pressure_sensitive: true,
        },
        BrushStyle {
            kind: BrushKind::Pen,
            color: Color::BLUE,
            width_pt: 2.2,
            pressure_sensitive: true,
        },
        BrushStyle {
            kind: BrushKind::Pen,
            color: Color::RED,
            width_pt: 2.2,
            pressure_sensitive: true,
        },
        BrushStyle {
            kind: BrushKind::Pencil,
            color: Color::PENCIL_GRAPHITE,
            width_pt: 2.6,
            pressure_sensitive: true,
        },
        BrushStyle {
            kind: BrushKind::HighlighterRound,
            color: Color::HIGHLIGHTER_YELLOW,
            width_pt: 16.0,
            pressure_sensitive: false,
        },
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn blackboard_auto_invert_flips_black_and_white_only() {
        let dark_inverted = Color::BLACK.effective_for_paper(PaperPolarity::Blackboard, true);
        assert!(!dark_inverted.is_dark());
        assert_eq!(dark_inverted.a, Color::BLACK.a);

        let chalk_on_light = Color::CHALK_WHITE.effective_for_paper(PaperPolarity::Light, true);
        assert!(
            !chalk_on_light.is_dark(),
            "light paper renders strokes literally"
        );

        let chalk_inverted =
            Color::CHALK_WHITE.effective_for_paper(PaperPolarity::Blackboard, true);
        assert!(chalk_inverted.is_dark());

        let blue_preserved = Color::BLUE.effective_for_paper(PaperPolarity::Blackboard, true);
        assert_eq!(blue_preserved, Color::BLUE);

        let literal = Color::BLACK.effective_for_paper(PaperPolarity::Blackboard, false);
        assert_eq!(literal, Color::BLACK);
    }
}

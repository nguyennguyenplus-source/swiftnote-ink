use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PaperStandard {
    A4,
    Letter,
    Legal,
    Square,
    Custom,
}

impl PaperStandard {
    /// Canonical typographic points (`72 pt = 1 inch`).
    #[inline]
    pub const fn dimensions_pt(self) -> (f64, f64) {
        match self {
            Self::A4 => (595.28, 841.89),
            Self::Letter => (612.0, 792.0),
            Self::Legal => (612.0, 1008.0),
            Self::Square | Self::Custom => (612.0, 612.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PaperTemplate {
    Blank,
    Ruled { spacing_pt: f32 },
    Math { spacing_pt: f32 },
    Graph,
    Dotted { spacing_pt: f32 },
    Isometric,
}

impl PaperTemplate {
    pub const fn ruled_10() -> Self {
        Self::Ruled { spacing_pt: 10.0 }
    }
    pub const fn ruled_15() -> Self {
        Self::Ruled { spacing_pt: 15.0 }
    }
    pub const fn ruled_20() -> Self {
        Self::Ruled { spacing_pt: 20.0 }
    }
    pub const fn ruled_30() -> Self {
        Self::Ruled { spacing_pt: 30.0 }
    }
    pub const fn ruled_40() -> Self {
        Self::Ruled { spacing_pt: 40.0 }
    }

    pub const fn math_10() -> Self {
        Self::Math { spacing_pt: 10.0 }
    }
    pub const fn math_15() -> Self {
        Self::Math { spacing_pt: 15.0 }
    }
    pub const fn math_20() -> Self {
        Self::Math { spacing_pt: 20.0 }
    }
    pub const fn math_30() -> Self {
        Self::Math { spacing_pt: 30.0 }
    }
    pub const fn math_40() -> Self {
        Self::Math { spacing_pt: 40.0 }
    }

    pub const fn dotted_10() -> Self {
        Self::Dotted { spacing_pt: 10.0 }
    }
    pub const fn dotted_20() -> Self {
        Self::Dotted { spacing_pt: 20.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PaperPolarity {
    #[default]
    Light,
    Blackboard,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageBackground {
    Procedural {
        template: PaperTemplate,
        polarity: PaperPolarity,
    },
    Pdf {
        asset_id: [u8; 32],
        page_index: usize,
        locked_placeholder: bool,
    },
    CustomImage {
        asset_id: [u8; 32],
    },
}

impl Default for PageBackground {
    fn default() -> Self {
        Self::Procedural {
            template: PaperTemplate::math_20(),
            polarity: PaperPolarity::Light,
        }
    }
}

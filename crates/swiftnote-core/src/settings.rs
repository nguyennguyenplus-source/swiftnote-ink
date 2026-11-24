use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiTheme {
    #[default]
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum InputMode {
    #[default]
    DigitizerPen,
    DigitizerPenNoPressure,
    FingerCapacitivePen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StabilizerPreset {
    Off,
    #[default]
    Light,
    Medium,
    Strong,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub ui_theme: UiTheme,
    pub input_mode: InputMode,
    pub stabilizer_preset: StabilizerPreset,
    pub back_to_pen_after_erase: bool,
    /// When true (default), near-black and near-white strokes auto-invert on Blackboard pages.
    pub auto_invert_monochrome_ink: bool,
    pub autosave_interval_secs: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ui_theme: UiTheme::Light,
            input_mode: InputMode::DigitizerPen,
            stabilizer_preset: StabilizerPreset::Light,
            back_to_pen_after_erase: false,
            auto_invert_monochrome_ink: true,
            autosave_interval_secs: 10,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_spec() {
        let s = AppSettings::default();
        assert!(s.auto_invert_monochrome_ink);
        assert_eq!(s.stabilizer_preset, StabilizerPreset::Light);
        assert!(!s.back_to_pen_after_erase);
        assert_eq!(s.autosave_interval_secs, 10);
    }
}

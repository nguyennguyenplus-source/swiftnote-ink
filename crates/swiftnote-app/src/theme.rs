use egui::epaint::Shadow;
use egui::{Color32, CornerRadius, Stroke, Style, Visuals};
use swiftnote_core::UiTheme;

/// Resolved theme mode used by the desktop chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

/// Solid-matte color tokens for Swift Note (strictly zero alpha translucency / blur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MattePalette {
    pub desk: Color32,
    pub surface: Color32,
    pub surface_subtle: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    pub border: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub danger: Color32,
    pub success: Color32,
}

impl MattePalette {
    pub const LIGHT: Self = Self {
        desk: Color32::from_rgb(0xEC, 0xEC, 0xEE),
        surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        surface_subtle: Color32::from_rgb(0xF4, 0xF4, 0xF6),
        surface_hover: Color32::from_rgb(0xE8, 0xE8, 0xEC),
        surface_active: Color32::from_rgb(0xDF, 0xDF, 0xE4),
        border: Color32::from_rgb(0xDC, 0xDC, 0xDE),
        text_primary: Color32::from_rgb(0x18, 0x18, 0x1B),
        text_secondary: Color32::from_rgb(0x52, 0x52, 0x5B),
        accent: Color32::from_rgb(0x00, 0x66, 0xFF),
        accent_soft: Color32::from_rgb(0xE5, 0xF0, 0xFF),
        danger: Color32::from_rgb(0xDC, 0x26, 0x26),
        success: Color32::from_rgb(0x16, 0xA3, 0x4A),
    };

    pub const DARK: Self = Self {
        desk: Color32::from_rgb(0x0E, 0x0E, 0x10),
        surface: Color32::from_rgb(0x18, 0x18, 0x1B),
        surface_subtle: Color32::from_rgb(0x22, 0x22, 0x26),
        surface_hover: Color32::from_rgb(0x2C, 0x2C, 0x32),
        surface_active: Color32::from_rgb(0x34, 0x34, 0x3C),
        border: Color32::from_rgb(0x2E, 0x2E, 0x33),
        text_primary: Color32::from_rgb(0xF4, 0xF4, 0xF5),
        text_secondary: Color32::from_rgb(0xA1, 0xA1, 0xAA),
        accent: Color32::from_rgb(0x3B, 0x82, 0xF6),
        accent_soft: Color32::from_rgb(0x1E, 0x2E, 0x4A),
        danger: Color32::from_rgb(0xEF, 0x44, 0x44),
        success: Color32::from_rgb(0x22, 0xC5, 0x5E),
    };

    pub const fn for_theme(theme: ResolvedTheme) -> Self {
        match theme {
            ResolvedTheme::Light => Self::LIGHT,
            ResolvedTheme::Dark => Self::DARK,
        }
    }

    /// Converts the desk backdrop to linear premultiplied RGBA floats for sRGB swapchain clears.
    pub fn desk_linear_rgba(self) -> [f32; 4] {
        fn srgb_to_linear(c: u8) -> f32 {
            let s = f32::from(c) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        [
            srgb_to_linear(self.desk.r()),
            srgb_to_linear(self.desk.g()),
            srgb_to_linear(self.desk.b()),
            1.0,
        ]
    }
}

/// Resolves `UiTheme::System` using the optional platform dark-mode hint.
pub fn resolve_theme(ui_theme: UiTheme, system_dark: Option<bool>) -> ResolvedTheme {
    match ui_theme {
        UiTheme::Light => ResolvedTheme::Light,
        UiTheme::Dark => ResolvedTheme::Dark,
        UiTheme::System => {
            if system_dark == Some(true) {
                ResolvedTheme::Dark
            } else {
                ResolvedTheme::Light
            }
        }
    }
}

/// Builds solid-opaque matte `egui::Visuals` with `1 px` hairline borders and zero blur/shadow.
pub fn build_matte_visuals(theme: ResolvedTheme) -> Visuals {
    let p = MattePalette::for_theme(theme);
    let mut visuals = match theme {
        ResolvedTheme::Light => Visuals::light(),
        ResolvedTheme::Dark => Visuals::dark(),
    };

    visuals.dark_mode = matches!(theme, ResolvedTheme::Dark);
    visuals.override_text_color = None;
    visuals.panel_fill = p.surface;
    visuals.window_fill = p.surface;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.window_highlight_topmost = false;
    visuals.faint_bg_color = p.surface_subtle;
    visuals.extreme_bg_color = p.surface_subtle;
    visuals.text_edit_bg_color = Some(p.surface_subtle);
    visuals.code_bg_color = p.surface_subtle;
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.danger;
    visuals.error_fg_color = p.danger;

    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.accent);

    let radius = CornerRadius::same(5);
    visuals.widgets.noninteractive.bg_fill = p.surface;
    visuals.widgets.noninteractive.weak_bg_fill = p.surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_primary);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.noninteractive.expansion = 0.0;

    visuals.widgets.inactive.bg_fill = p.surface_subtle;
    visuals.widgets.inactive.weak_bg_fill = p.surface;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text_primary);
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.inactive.expansion = 0.0;

    visuals.widgets.hovered.bg_fill = p.surface_hover;
    visuals.widgets.hovered.weak_bg_fill = p.surface_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text_primary);
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = p.surface_active;
    visuals.widgets.active.weak_bg_fill = p.surface_active;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, p.text_primary);
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = p.surface_hover;
    visuals.widgets.open.weak_bg_fill = p.surface_hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, p.text_primary);
    visuals.widgets.open.corner_radius = radius;
    visuals.widgets.open.expansion = 0.0;

    visuals
}

/// Applies the solid-matte visuals and spacing tokens to `egui::Context`.
pub fn apply_matte_theme(ctx: &egui::Context, theme: ResolvedTheme) {
    let mut style = Style {
        visuals: build_matte_visuals(theme),
        ..Style::default()
    };
    style.spacing.button_padding = egui::vec2(8.0, 5.0);
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(12);
    let egui_theme = match theme {
        ResolvedTheme::Light => egui::Theme::Light,
        ResolvedTheme::Dark => egui::Theme::Dark,
    };
    ctx.set_style_of(egui_theme, style);
    ctx.set_theme(egui_theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matte_theme_has_zero_translucent_panels_or_blur() {
        for theme in [ResolvedTheme::Light, ResolvedTheme::Dark] {
            let v = build_matte_visuals(theme);
            assert_eq!(v.window_fill.a(), 255);
            assert_eq!(v.panel_fill.a(), 255);
            assert_eq!(v.extreme_bg_color.a(), 255);
            assert_eq!(v.faint_bg_color.a(), 255);
            assert_eq!(v.selection.bg_fill.a(), 255);
            assert_eq!(v.widgets.noninteractive.bg_fill.a(), 255);
            assert_eq!(v.widgets.inactive.bg_fill.a(), 255);
            assert_eq!(v.widgets.hovered.bg_fill.a(), 255);
            assert_eq!(v.widgets.active.bg_fill.a(), 255);
            assert_eq!(v.widgets.open.bg_fill.a(), 255);
            assert_eq!(v.window_shadow, Shadow::NONE);
            assert_eq!(v.popup_shadow, Shadow::NONE);
        }
    }
}

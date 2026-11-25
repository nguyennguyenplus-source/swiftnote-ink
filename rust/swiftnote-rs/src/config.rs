use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLayoutMode {
    PureTopBar,
    CanvasPill,
    ZenBottomDock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutBinding {
    pub action_id: String,
    pub label: String,
    /// Single-key name normalized lowercase (e.g. "e", "l", "b", "z", "1", "tab")
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PenKind {
    Pen,
    Highlighter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineStyleKind {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PenSlotConfig {
    pub id: u32,
    pub label: String,
    pub kind: PenKind,
    /// RGB color packed as 0xRRGGBB
    pub color_rgb: u32,
    /// Stroke width in points (Xournal++ tool thickness)
    pub width_pt: f64,
    pub pressure_sensitive: bool,
    pub line_style: LineStyleKind,
    pub ink_to_shape: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackboardConfig {
    /// Whether blackboard mode is currently active by default / last session
    pub enabled: bool,
    /// Automatically invert black/dark-gray <-> white/light-gray ink on display
    pub auto_invert_bw_ink: bool,
    /// Matte chalkboard background color (default: #16181D)
    pub surface_rgb: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseUpConfig {
    pub zoom_factor: f64,
    pub auto_advance: bool,
    pub panel_height_px: u32,
    pub margin_trigger_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwiftConfig {
    pub schema_version: u32,
    pub theme: ThemeMode,
    pub ui_layout: UiLayoutMode,
    pub active_pen_slot: usize,
    pub pen_slots: Vec<PenSlotConfig>,
    pub shortcuts: Vec<ShortcutBinding>,
    pub eraser_auto_return: bool,
    pub blackboard: BlackboardConfig,
    pub close_up: CloseUpConfig,
    pub library_dir: String,
    pub default_paper_template: String,
    pub recent_files: Vec<String>,
}

impl Default for BlackboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_invert_bw_ink: true,
            surface_rgb: 0x16181D,
        }
    }
}

impl Default for CloseUpConfig {
    fn default() -> Self {
        Self {
            zoom_factor: 2.5,
            auto_advance: true,
            panel_height_px: 260,
            margin_trigger_ratio: 0.78,
        }
    }
}

impl Default for SwiftConfig {
    fn default() -> Self {
        Self {
            schema_version: 2,
            theme: ThemeMode::Light,
            ui_layout: UiLayoutMode::CanvasPill,
            active_pen_slot: 0,
            pen_slots: default_pen_slots(),
            shortcuts: default_shortcuts(),
            eraser_auto_return: true,
            blackboard: BlackboardConfig::default(),
            close_up: CloseUpConfig::default(),
            library_dir: default_library_dir().to_string_lossy().into_owned(),
            default_paper_template: "ruled".to_string(),
            recent_files: Vec::new(),
        }
    }
}

pub fn default_shortcuts() -> Vec<ShortcutBinding> {
    let pairs: &[(&str, &str, &str)] = &[
        ("pen_1", "Bút số 1", "1"),
        ("pen_2", "Bút số 2", "2"),
        ("pen_3", "Bút số 3", "3"),
        ("pen_4", "Bút số 4", "4"),
        ("pen_5", "Dạ quang / Bút số 5", "5"),
        ("eraser", "Tẩy (bấm lại để về bút)", "e"),
        ("lasso", "Chọn vùng tự do (Lasso)", "l"),
        ("box_select", "Chọn hình chữ nhật", "s"),
        ("shapes", "Hình học / Ink-to-Shape", "g"),
        ("text", "Văn bản (Text Box)", "t"),
        ("ruler", "Thước ê-ke & Đo góc", "r"),
        ("compass", "Compa", "c"),
        ("spacer", "Dịch khoảng trống dọc", "v"),
        ("laser", "Con trỏ Laser", "p"),
        ("hand", "Chế độ cuộn tay (Hand)", "h"),
        ("blackboard", "Bật / tắt Bảng đen", "b"),
        ("close_up", "Vùng viết phóng to", "z"),
        ("new_page", "Thêm trang mới", "n"),
        ("toggle_theme", "Đổi Light / Dark Mode", "d"),
        ("focus_mode", "Ẩn / hiện thanh công cụ", "tab"),
        ("fullscreen", "Toàn màn hình", "f"),
    ];
    pairs
        .iter()
        .map(|(id, label, key)| ShortcutBinding {
            action_id: (*id).to_string(),
            label: (*label).to_string(),
            key: (*key).to_string(),
        })
        .collect()
}

pub fn default_pen_slots() -> Vec<PenSlotConfig> {
    vec![
        PenSlotConfig {
            id: 1,
            label: "Bút đen".to_string(),
            kind: PenKind::Pen,
            color_rgb: 0x111111,
            width_pt: 1.41,
            pressure_sensitive: true,
            line_style: LineStyleKind::Solid,
            ink_to_shape: false,
        },
        PenSlotConfig {
            id: 2,
            label: "Bút xanh".to_string(),
            kind: PenKind::Pen,
            color_rgb: 0x0A58CA,
            width_pt: 1.41,
            pressure_sensitive: true,
            line_style: LineStyleKind::Solid,
            ink_to_shape: false,
        },
        PenSlotConfig {
            id: 3,
            label: "Bút đỏ".to_string(),
            kind: PenKind::Pen,
            color_rgb: 0xD92D20,
            width_pt: 1.41,
            pressure_sensitive: true,
            line_style: LineStyleKind::Solid,
            ink_to_shape: false,
        },
        PenSlotConfig {
            id: 4,
            label: "Bút nhấn".to_string(),
            kind: PenKind::Pen,
            color_rgb: 0x15803D,
            width_pt: 2.26,
            pressure_sensitive: true,
            line_style: LineStyleKind::Solid,
            ink_to_shape: false,
        },
        PenSlotConfig {
            id: 5,
            label: "Dạ quang".to_string(),
            kind: PenKind::Highlighter,
            color_rgb: 0xFACC15,
            width_pt: 14.0,
            pressure_sensitive: false,
            line_style: LineStyleKind::Solid,
            ink_to_shape: false,
        },
    ]
}

pub fn default_config_path() -> PathBuf {
    if let Some(proj) = ProjectDirs::from("io.github", "swiftnote", "SwiftNote") {
        return proj.config_dir().join("swiftnote-config.json");
    }
    PathBuf::from("swiftnote-config.json")
}

pub fn default_library_dir() -> PathBuf {
    if let Some(user_dirs) = directories::UserDirs::new() {
        if let Some(docs) = user_dirs.document_dir() {
            return docs.join("Swift note");
        }
        return user_dirs.home_dir().join("Swift note");
    }
    PathBuf::from("Swift note")
}

/// Loads `SwiftConfig` from disk, falling back cleanly to defaults if missing or corrupted.
pub fn load_config(path: &Path) -> SwiftConfig {
    let Ok(text) = fs::read_to_string(path) else {
        return SwiftConfig::default();
    };
    let mut cfg: SwiftConfig = serde_json::from_str(&text).unwrap_or_default();
    sanitize_config(&mut cfg);
    cfg
}

pub fn sanitize_config(cfg: &mut SwiftConfig) {
    if cfg.pen_slots.is_empty() {
        cfg.pen_slots = default_pen_slots();
    }
    if cfg.active_pen_slot >= cfg.pen_slots.len() {
        cfg.active_pen_slot = 0;
    }
    // Ensure every default shortcut action exists even when migrating older config files
    let defaults = default_shortcuts();
    if cfg.shortcuts.is_empty() {
        cfg.shortcuts = defaults;
    } else {
        for def in defaults {
            if !cfg.shortcuts.iter().any(|s| s.action_id == def.action_id) {
                cfg.shortcuts.push(def);
            }
        }
    }
    for sc in &mut cfg.shortcuts {
        sc.key = sc.key.trim().to_ascii_lowercase();
    }
    for slot in &mut cfg.pen_slots {
        slot.width_pt = slot.width_pt.clamp(0.3, 48.0);
        slot.color_rgb &= 0x00FF_FFFF;
    }
    cfg.close_up.zoom_factor = cfg.close_up.zoom_factor.clamp(1.5, 5.0);
    cfg.close_up.panel_height_px = cfg.close_up.panel_height_px.clamp(160, 600);
    cfg.close_up.margin_trigger_ratio = cfg.close_up.margin_trigger_ratio.clamp(0.55, 0.92);
    cfg.blackboard.surface_rgb &= 0x00FF_FFFF;
}

/// Atomically persists `SwiftConfig` (writes temp file in same directory, flushes, then renames).
pub fn save_config(path: &Path, cfg: &SwiftConfig) -> Result<()> {
    let mut clean = cfg.clone();
    sanitize_config(&mut clean);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let payload = serde_json::to_string_pretty(&clean)?;
    let tmp_path = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(payload.as_bytes())?;
        file.sync_all()?;
    }

    // On Windows, fs::rename fails if target exists in some older environments; remove fallback handles it safely.
    if fs::rename(&tmp_path, path).is_err() {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        fs::rename(&tmp_path, path)?;
    }
    Ok(())
}

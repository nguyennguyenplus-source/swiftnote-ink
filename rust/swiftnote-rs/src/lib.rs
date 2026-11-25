pub mod config;
pub mod error;
pub mod library;
pub mod media;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::{Mutex, OnceLock};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnStatus {
    Ok = 0,
    ErrNullPointer = 1,
    ErrInvalidUtf8 = 2,
    ErrIo = 3,
    ErrParse = 4,
    ErrMedia = 5,
    ErrOutOfRange = 6,
    ErrPanic = 99,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnThemeMode {
    Light = 0,
    Dark = 1,
    System = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnUiLayout {
    PureTopBar = 0,
    CanvasPill = 1,
    ZenBottomDock = 2,
}

#[repr(C)]
pub struct SnShortcutEntry {
    pub action_id: [c_char; 48],
    pub label: [c_char; 96],
    pub key: [c_char; 24],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnPenKind {
    Pen = 0,
    Highlighter = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnLineStyle {
    Solid = 0,
    Dashed = 1,
    Dotted = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnNoteKind {
    Xopp = 0,
    Xoj = 1,
    Pdf = 2,
}

#[repr(C)]
pub struct SnByteBuffer {
    pub data: *mut u8,
    pub len: usize,
    pub cap: usize,
}

#[repr(C)]
pub struct SnPenSlot {
    pub id: u32,
    pub label: [c_char; 64],
    pub kind: SnPenKind,
    pub color_rgb: u32,
    pub width_pt: f64,
    pub pressure_sensitive: bool,
    pub line_style: SnLineStyle,
    pub ink_to_shape: bool,
}

#[repr(C)]
pub struct SnAppSettings {
    pub theme: SnThemeMode,
    pub ui_layout: SnUiLayout,
    pub active_pen_slot: u32,
    pub pen_slot_count: u32,
    pub eraser_auto_return: bool,
    pub blackboard_enabled: bool,
    pub blackboard_auto_invert_bw_ink: bool,
    pub blackboard_surface_rgb: u32,
    pub close_up_zoom_factor: f64,
    pub close_up_auto_advance: bool,
    pub close_up_panel_height_px: u32,
    pub close_up_margin_trigger_ratio: f64,
    pub library_dir: [c_char; 512],
    pub default_paper_template: [c_char; 64],
}

#[repr(C)]
pub struct SnNoteItem {
    pub title: [c_char; 256],
    pub path: [c_char; 512],
    pub folder: [c_char; 256],
    pub kind: SnNoteKind,
    pub modified_epoch_secs: u64,
    pub size_bytes: u64,
    pub page_count: u32,
    pub has_preview: bool,
}

#[repr(C)]
pub struct SnNoteList {
    pub items: *mut SnNoteItem,
    pub len: usize,
    pub cap: usize,
}

struct GlobalState {
    path: PathBuf,
    cfg: config::SwiftConfig,
}

static STATE: OnceLock<Mutex<GlobalState>> = OnceLock::new();

fn global_state() -> &'static Mutex<GlobalState> {
    STATE.get_or_init(|| {
        let path = config::default_config_path();
        let cfg = config::load_config(&path);
        Mutex::new(GlobalState { path, cfg })
    })
}

fn write_fixed_cstr<const N: usize>(dst: &mut [c_char; N], s: &str) {
    dst.fill(0);
    let bytes = s.as_bytes();
    let max_len = N.saturating_sub(1);
    // Avoid splitting a multi-byte UTF-8 sequence
    let mut copy_len = bytes.len().min(max_len);
    while copy_len > 0 && !s.is_char_boundary(copy_len) {
        copy_len -= 1;
    }
    for (i, &b) in bytes[..copy_len].iter().enumerate() {
        dst[i] = b as c_char;
    }
}

fn read_fixed_cstr<const N: usize>(src: &[c_char; N]) -> String {
    let len = src.iter().position(|&c| c == 0).unwrap_or(N);
    let bytes: Vec<u8> = src[..len].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn to_ffi_slot(slot: &config::PenSlotConfig) -> SnPenSlot {
    let mut label = [0 as c_char; 64];
    write_fixed_cstr(&mut label, &slot.label);
    SnPenSlot {
        id: slot.id,
        label,
        kind: match slot.kind {
            config::PenKind::Pen => SnPenKind::Pen,
            config::PenKind::Highlighter => SnPenKind::Highlighter,
        },
        color_rgb: slot.color_rgb,
        width_pt: slot.width_pt,
        pressure_sensitive: slot.pressure_sensitive,
        line_style: match slot.line_style {
            config::LineStyleKind::Solid => SnLineStyle::Solid,
            config::LineStyleKind::Dashed => SnLineStyle::Dashed,
            config::LineStyleKind::Dotted => SnLineStyle::Dotted,
        },
        ink_to_shape: slot.ink_to_shape,
    }
}

fn from_ffi_slot(slot: &SnPenSlot) -> config::PenSlotConfig {
    config::PenSlotConfig {
        id: slot.id,
        label: read_fixed_cstr(&slot.label),
        kind: match slot.kind {
            SnPenKind::Pen => config::PenKind::Pen,
            SnPenKind::Highlighter => config::PenKind::Highlighter,
        },
        color_rgb: slot.color_rgb & 0x00FF_FFFF,
        width_pt: slot.width_pt.clamp(0.3, 48.0),
        pressure_sensitive: slot.pressure_sensitive,
        line_style: match slot.line_style {
            SnLineStyle::Solid => config::LineStyleKind::Solid,
            SnLineStyle::Dashed => config::LineStyleKind::Dashed,
            SnLineStyle::Dotted => config::LineStyleKind::Dotted,
        },
        ink_to_shape: slot.ink_to_shape,
    }
}

fn to_ffi_settings(cfg: &config::SwiftConfig) -> SnAppSettings {
    let mut library_dir = [0 as c_char; 512];
    write_fixed_cstr(&mut library_dir, &cfg.library_dir);
    let mut default_paper_template = [0 as c_char; 64];
    write_fixed_cstr(&mut default_paper_template, &cfg.default_paper_template);
    SnAppSettings {
        theme: match cfg.theme {
            config::ThemeMode::Light => SnThemeMode::Light,
            config::ThemeMode::Dark => SnThemeMode::Dark,
            config::ThemeMode::System => SnThemeMode::System,
        },
        ui_layout: match cfg.ui_layout {
            config::UiLayoutMode::PureTopBar => SnUiLayout::PureTopBar,
            config::UiLayoutMode::CanvasPill => SnUiLayout::CanvasPill,
            config::UiLayoutMode::ZenBottomDock => SnUiLayout::ZenBottomDock,
        },
        active_pen_slot: cfg.active_pen_slot as u32,
        pen_slot_count: cfg.pen_slots.len() as u32,
        eraser_auto_return: cfg.eraser_auto_return,
        blackboard_enabled: cfg.blackboard.enabled,
        blackboard_auto_invert_bw_ink: cfg.blackboard.auto_invert_bw_ink,
        blackboard_surface_rgb: cfg.blackboard.surface_rgb,
        close_up_zoom_factor: cfg.close_up.zoom_factor,
        close_up_auto_advance: cfg.close_up.auto_advance,
        close_up_panel_height_px: cfg.close_up.panel_height_px,
        close_up_margin_trigger_ratio: cfg.close_up.margin_trigger_ratio,
        library_dir,
        default_paper_template,
    }
}

impl SnByteBuffer {
    fn from_vec(mut v: Vec<u8>) -> Self {
        let buf = Self {
            data: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        };
        std::mem::forget(v);
        buf
    }
}

fn map_error(err: &error::SwiftNoteError) -> SnStatus {
    match err {
        error::SwiftNoteError::Io(_) => SnStatus::ErrIo,
        error::SwiftNoteError::Json(_) => SnStatus::ErrParse,
        error::SwiftNoteError::Media(_) => SnStatus::ErrMedia,
        error::SwiftNoteError::InvalidInput => SnStatus::ErrInvalidUtf8,
    }
}

unsafe fn read_c_str<'a>(ptr: *const c_char) -> std::result::Result<&'a str, SnStatus> {
    if ptr.is_null() {
        return Err(SnStatus::ErrNullPointer);
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map_err(|_| SnStatus::ErrInvalidUtf8)
}

unsafe fn write_out_string(out_ptr: *mut *mut c_char, value: String) -> SnStatus {
    if out_ptr.is_null() {
        return SnStatus::ErrNullPointer;
    }
    match CString::new(value) {
        Ok(cstr) => {
            *out_ptr = cstr.into_raw();
            SnStatus::Ok
        }
        Err(_) => SnStatus::ErrInvalidUtf8,
    }
}

#[no_mangle]
pub extern "C" fn sn_version() -> *const c_char {
    c"0.1.0-beta.1".as_ptr()
}

/// # Safety
/// `ptr` must be a pointer returned by a `sn_*` function or null.
#[no_mangle]
pub unsafe extern "C" fn sn_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// # Safety
/// `buf` must have been produced by a `sn_*` function.
#[no_mangle]
pub unsafe extern "C" fn sn_buffer_free(buf: SnByteBuffer) {
    if !buf.data.is_null() && buf.cap > 0 {
        drop(Vec::from_raw_parts(buf.data, buf.len, buf.cap));
    }
}

/// # Safety
/// `list` must have been produced by `sn_library_scan_list`.
#[no_mangle]
pub unsafe extern "C" fn sn_note_list_free(list: SnNoteList) {
    if !list.items.is_null() && list.cap > 0 {
        drop(Vec::from_raw_parts(list.items, list.len, list.cap));
    }
}

/// # Safety
/// `out_path` must be a valid non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn sn_config_default_path(out_path: *mut *mut c_char) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let p = config::default_config_path();
        write_out_string(out_path, p.to_string_lossy().into_owned())
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `out_json` must be non-null. `path_utf8` may be null (uses default config path).
#[no_mangle]
pub unsafe extern "C" fn sn_config_load_json(
    path_utf8: *const c_char,
    out_json: *mut *mut c_char,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_json.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let path_buf = if path_utf8.is_null() {
            config::default_config_path()
        } else {
            match read_c_str(path_utf8) {
                Ok(s) if !s.is_empty() => Path::new(s).to_path_buf(),
                Ok(_) => config::default_config_path(),
                Err(code) => return code,
            }
        };
        let cfg = config::load_config(&path_buf);
        match serde_json::to_string(&cfg) {
            Ok(json) => write_out_string(out_json, json),
            Err(_) => SnStatus::ErrParse,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `json_utf8` must be a valid null-terminated UTF-8 string. `path_utf8` may be null.
#[no_mangle]
pub unsafe extern "C" fn sn_config_save_json(
    path_utf8: *const c_char,
    json_utf8: *const c_char,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let json = match read_c_str(json_utf8) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let path_buf = if path_utf8.is_null() {
            config::default_config_path()
        } else {
            match read_c_str(path_utf8) {
                Ok(s) if !s.is_empty() => Path::new(s).to_path_buf(),
                Ok(_) => config::default_config_path(),
                Err(code) => return code,
            }
        };
        let cfg: config::SwiftConfig = match serde_json::from_str(json) {
            Ok(c) => c,
            Err(_) => return SnStatus::ErrParse,
        };
        match config::save_config(&path_buf, &cfg) {
            Ok(()) => SnStatus::Ok,
            Err(e) => map_error(&e),
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `path_utf8` may be null (uses default config path).
#[no_mangle]
pub unsafe extern "C" fn sn_state_init(path_utf8: *const c_char) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let path_buf = if path_utf8.is_null() {
            config::default_config_path()
        } else {
            match read_c_str(path_utf8) {
                Ok(s) if !s.is_empty() => Path::new(s).to_path_buf(),
                Ok(_) => config::default_config_path(),
                Err(code) => return code,
            }
        };
        let cfg = config::load_config(&path_buf);
        match global_state().lock() {
            Ok(mut guard) => {
                guard.path = path_buf;
                guard.cfg = cfg;
                SnStatus::Ok
            }
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `out_settings` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_state_get_settings(out_settings: *mut SnAppSettings) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_settings.is_null() {
            return SnStatus::ErrNullPointer;
        }
        match global_state().lock() {
            Ok(guard) => {
                *out_settings = to_ffi_settings(&guard.cfg);
                SnStatus::Ok
            }
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `settings` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_state_set_settings(settings: *const SnAppSettings) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if settings.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let s = &*settings;
        match global_state().lock() {
            Ok(mut guard) => {
                guard.cfg.theme = match s.theme {
                    SnThemeMode::Light => config::ThemeMode::Light,
                    SnThemeMode::Dark => config::ThemeMode::Dark,
                    SnThemeMode::System => config::ThemeMode::System,
                };
                guard.cfg.ui_layout = match s.ui_layout {
                    SnUiLayout::PureTopBar => config::UiLayoutMode::PureTopBar,
                    SnUiLayout::CanvasPill => config::UiLayoutMode::CanvasPill,
                    SnUiLayout::ZenBottomDock => config::UiLayoutMode::ZenBottomDock,
                };
                guard.cfg.active_pen_slot = s.active_pen_slot as usize;
                guard.cfg.eraser_auto_return = s.eraser_auto_return;
                guard.cfg.blackboard.enabled = s.blackboard_enabled;
                guard.cfg.blackboard.auto_invert_bw_ink = s.blackboard_auto_invert_bw_ink;
                guard.cfg.blackboard.surface_rgb = s.blackboard_surface_rgb;
                guard.cfg.close_up.zoom_factor = s.close_up_zoom_factor;
                guard.cfg.close_up.auto_advance = s.close_up_auto_advance;
                guard.cfg.close_up.panel_height_px = s.close_up_panel_height_px;
                guard.cfg.close_up.margin_trigger_ratio = s.close_up_margin_trigger_ratio;
                let lib_dir = read_fixed_cstr(&s.library_dir);
                if !lib_dir.is_empty() {
                    guard.cfg.library_dir = lib_dir;
                }
                let tpl = read_fixed_cstr(&s.default_paper_template);
                if !tpl.is_empty() {
                    guard.cfg.default_paper_template = tpl;
                }
                config::sanitize_config(&mut guard.cfg);
                SnStatus::Ok
            }
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

#[no_mangle]
pub extern "C" fn sn_state_pen_slot_count() -> u32 {
    catch_unwind(|| {
        global_state()
            .lock()
            .map(|g| g.cfg.pen_slots.len() as u32)
            .unwrap_or(0)
    })
    .unwrap_or(0)
}

/// # Safety
/// `out_slot` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_state_get_pen_slot(
    index: u32,
    out_slot: *mut SnPenSlot,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_slot.is_null() {
            return SnStatus::ErrNullPointer;
        }
        match global_state().lock() {
            Ok(guard) => match guard.cfg.pen_slots.get(index as usize) {
                Some(slot) => {
                    *out_slot = to_ffi_slot(slot);
                    SnStatus::Ok
                }
                None => SnStatus::ErrOutOfRange,
            },
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `slot` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_state_set_pen_slot(index: u32, slot: *const SnPenSlot) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if slot.is_null() {
            return SnStatus::ErrNullPointer;
        }
        match global_state().lock() {
            Ok(mut guard) => match guard.cfg.pen_slots.get_mut(index as usize) {
                Some(dst) => {
                    *dst = from_ffi_slot(&*slot);
                    SnStatus::Ok
                }
                None => SnStatus::ErrOutOfRange,
            },
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `slot` must be non-null. `out_index` may be null.
#[no_mangle]
pub unsafe extern "C" fn sn_state_add_pen_slot(
    slot: *const SnPenSlot,
    out_index: *mut u32,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if slot.is_null() {
            return SnStatus::ErrNullPointer;
        }
        match global_state().lock() {
            Ok(mut guard) => {
                if guard.cfg.pen_slots.len() >= 10 {
                    return SnStatus::ErrOutOfRange;
                }
                let mut item = from_ffi_slot(&*slot);
                item.id = guard
                    .cfg
                    .pen_slots
                    .iter()
                    .map(|s| s.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                guard.cfg.pen_slots.push(item);
                let idx = (guard.cfg.pen_slots.len() - 1) as u32;
                if !out_index.is_null() {
                    *out_index = idx;
                }
                SnStatus::Ok
            }
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

#[no_mangle]
pub extern "C" fn sn_state_remove_pen_slot(index: u32) -> SnStatus {
    catch_unwind(|| match global_state().lock() {
        Ok(mut guard) => {
            let idx = index as usize;
            if guard.cfg.pen_slots.len() <= 1 || idx >= guard.cfg.pen_slots.len() {
                return SnStatus::ErrOutOfRange;
            }
            guard.cfg.pen_slots.remove(idx);
            config::sanitize_config(&mut guard.cfg);
            SnStatus::Ok
        }
        Err(_) => SnStatus::ErrPanic,
    })
    .unwrap_or(SnStatus::ErrPanic)
}

#[no_mangle]
pub extern "C" fn sn_state_shortcut_count() -> u32 {
    catch_unwind(|| {
        global_state()
            .lock()
            .map(|g| g.cfg.shortcuts.len() as u32)
            .unwrap_or(0)
    })
    .unwrap_or(0)
}

/// # Safety
/// `out_entry` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_state_get_shortcut(
    index: u32,
    out_entry: *mut SnShortcutEntry,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_entry.is_null() {
            return SnStatus::ErrNullPointer;
        }
        match global_state().lock() {
            Ok(guard) => match guard.cfg.shortcuts.get(index as usize) {
                Some(sc) => {
                    let mut action_id = [0 as c_char; 48];
                    let mut label = [0 as c_char; 96];
                    let mut key = [0 as c_char; 24];
                    write_fixed_cstr(&mut action_id, &sc.action_id);
                    write_fixed_cstr(&mut label, &sc.label);
                    write_fixed_cstr(&mut key, &sc.key);
                    *out_entry = SnShortcutEntry {
                        action_id,
                        label,
                        key,
                    };
                    SnStatus::Ok
                }
                None => SnStatus::ErrOutOfRange,
            },
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `action_id_utf8` and `key_utf8` must be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn sn_state_set_shortcut_key(
    action_id_utf8: *const c_char,
    key_utf8: *const c_char,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let action_id = match read_c_str(action_id_utf8) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let raw_key = match read_c_str(key_utf8) {
            Ok(s) => s.trim().to_ascii_lowercase(),
            Err(code) => return code,
        };
        match global_state().lock() {
            Ok(mut guard) => {
                // Clear any existing action holding the same non-empty key to avoid conflicts
                if !raw_key.is_empty() {
                    for item in &mut guard.cfg.shortcuts {
                        if item.action_id != action_id && item.key == raw_key {
                            item.key.clear();
                        }
                    }
                }
                if let Some(target) = guard
                    .cfg
                    .shortcuts
                    .iter_mut()
                    .find(|s| s.action_id == action_id)
                {
                    target.key = raw_key;
                    SnStatus::Ok
                } else {
                    SnStatus::ErrOutOfRange
                }
            }
            Err(_) => SnStatus::ErrPanic,
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

#[no_mangle]
pub extern "C" fn sn_state_reset_shortcuts() -> SnStatus {
    catch_unwind(|| match global_state().lock() {
        Ok(mut guard) => {
            guard.cfg.shortcuts = config::default_shortcuts();
            SnStatus::Ok
        }
        Err(_) => SnStatus::ErrPanic,
    })
    .unwrap_or(SnStatus::ErrPanic)
}

#[no_mangle]
pub extern "C" fn sn_state_save() -> SnStatus {
    catch_unwind(|| match global_state().lock() {
        Ok(guard) => match config::save_config(&guard.path, &guard.cfg) {
            Ok(()) => SnStatus::Ok,
            Err(e) => map_error(&e),
        },
        Err(_) => SnStatus::ErrPanic,
    })
    .unwrap_or(SnStatus::ErrPanic)
}

#[no_mangle]
pub extern "C" fn sn_invert_bw_ink_rgb(rgb24: u32) -> u32 {
    media::invert_bw_ink_rgb(rgb24)
}

/// # Safety
/// `png_bytes` must be valid for `png_len` bytes, `out_png` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_normalize_clipboard_png(
    png_bytes: *const u8,
    png_len: usize,
    out_png: *mut SnByteBuffer,
    out_width: *mut u32,
    out_height: *mut u32,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if png_bytes.is_null() || out_png.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let input = slice::from_raw_parts(png_bytes, png_len);
        match media::normalize_clipboard_png(input) {
            Ok((encoded, w, h)) => {
                *out_png = SnByteBuffer::from_vec(encoded);
                if !out_width.is_null() {
                    *out_width = w;
                }
                if !out_height.is_null() {
                    *out_height = h;
                }
                SnStatus::Ok
            }
            Err(e) => map_error(&e),
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `pixels` must be valid for `pixels_len` bytes, `out_png` must be non-null.
#[no_mangle]
pub unsafe extern "C" fn sn_pixels_to_lossless_png(
    pixels: *const u8,
    pixels_len: usize,
    width: u32,
    height: u32,
    stride: u32,
    is_bgra: bool,
    fix_all_zero_alpha: bool,
    out_png: *mut SnByteBuffer,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if pixels.is_null() || out_png.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let input = slice::from_raw_parts(pixels, pixels_len);
        match media::pixels_to_lossless_png(
            input,
            width,
            height,
            stride,
            is_bgra,
            fix_all_zero_alpha,
        ) {
            Ok(encoded) => {
                *out_png = SnByteBuffer::from_vec(encoded);
                SnStatus::Ok
            }
            Err(e) => map_error(&e),
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `root_dir_utf8` and `out_json` must be valid non-null pointers. `query_utf8` may be null.
#[no_mangle]
pub unsafe extern "C" fn sn_library_scan_json(
    root_dir_utf8: *const c_char,
    query_utf8: *const c_char,
    out_json: *mut *mut c_char,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_json.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let root_str = match read_c_str(root_dir_utf8) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let query_str = if query_utf8.is_null() {
            ""
        } else {
            match read_c_str(query_utf8) {
                Ok(s) => s,
                Err(code) => return code,
            }
        };
        match library::scan_library(Path::new(root_str), query_str) {
            Ok(res) => match serde_json::to_string(&res) {
                Ok(json) => write_out_string(out_json, json),
                Err(_) => SnStatus::ErrParse,
            },
            Err(e) => map_error(&e),
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `root_dir_utf8` and `out_list` must be valid non-null pointers. `query_utf8` may be null.
#[no_mangle]
pub unsafe extern "C" fn sn_library_scan_list(
    root_dir_utf8: *const c_char,
    query_utf8: *const c_char,
    out_list: *mut SnNoteList,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_list.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let root_str = match read_c_str(root_dir_utf8) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let query_str = if query_utf8.is_null() {
            ""
        } else {
            match read_c_str(query_utf8) {
                Ok(s) => s,
                Err(code) => return code,
            }
        };
        match library::scan_library(Path::new(root_str), query_str) {
            Ok(res) => {
                let mut items: Vec<SnNoteItem> = res
                    .notes
                    .into_iter()
                    .map(|n| {
                        let mut title = [0 as c_char; 256];
                        let mut path = [0 as c_char; 512];
                        let mut folder = [0 as c_char; 256];
                        write_fixed_cstr(&mut title, &n.title);
                        write_fixed_cstr(&mut path, &n.path);
                        write_fixed_cstr(&mut folder, &n.folder);
                        SnNoteItem {
                            title,
                            path,
                            folder,
                            kind: match n.kind {
                                library::NoteFileKind::Xopp => SnNoteKind::Xopp,
                                library::NoteFileKind::Xoj => SnNoteKind::Xoj,
                                library::NoteFileKind::Pdf => SnNoteKind::Pdf,
                            },
                            modified_epoch_secs: n.modified_epoch_secs,
                            size_bytes: n.size_bytes,
                            page_count: n.page_count,
                            has_preview: n.has_preview,
                        }
                    })
                    .collect();
                let list = SnNoteList {
                    items: items.as_mut_ptr(),
                    len: items.len(),
                    cap: items.capacity(),
                };
                std::mem::forget(items);
                *out_list = list;
                SnStatus::Ok
            }
            Err(e) => map_error(&e),
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

/// # Safety
/// `file_path_utf8` and `out_png` must be valid non-null pointers.
#[no_mangle]
pub unsafe extern "C" fn sn_library_extract_xopp_preview(
    file_path_utf8: *const c_char,
    out_png: *mut SnByteBuffer,
) -> SnStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_png.is_null() {
            return SnStatus::ErrNullPointer;
        }
        let path_str = match read_c_str(file_path_utf8) {
            Ok(s) => s,
            Err(code) => return code,
        };
        match library::extract_xopp_preview(Path::new(path_str)) {
            Ok(bytes) => {
                *out_png = SnByteBuffer::from_vec(bytes);
                SnStatus::Ok
            }
            Err(e) => map_error(&e),
        }
    }))
    .unwrap_or(SnStatus::ErrPanic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn config_roundtrip_and_atomic_save() {
        let dir = tempfile::tempdir().ok();
        assert!(dir.is_some());
        if let Some(dir) = dir {
            let path = dir.path().join("nested").join("swiftnote.json");
            let cfg = config::SwiftConfig {
                theme: config::ThemeMode::Dark,
                eraser_auto_return: true,
                blackboard: config::BlackboardConfig {
                    enabled: true,
                    auto_invert_bw_ink: false,
                    ..Default::default()
                },
                ..Default::default()
            };

            assert!(config::save_config(&path, &cfg).is_ok());
            let loaded = config::load_config(&path);
            assert_eq!(loaded, cfg);
        }
    }

    #[test]
    fn config_recovers_cleanly_from_corrupt_file() {
        let dir = tempfile::tempdir().ok();
        assert!(dir.is_some());
        if let Some(dir) = dir {
            let path = dir.path().join("corrupt.json");
            assert!(std::fs::write(&path, "{not-valid-json").is_ok());
            let loaded = config::load_config(&path);
            assert_eq!(loaded.pen_slots.len(), 5);
        }
    }

    #[test]
    fn blackboard_inverts_bw_and_preserves_colors() {
        assert_eq!(media::invert_bw_ink_rgb(0x000000), 0xFFFFFF);
        assert_eq!(media::invert_bw_ink_rgb(0xFFFFFF), 0x000000);
        assert_eq!(media::invert_bw_ink_rgb(0x111111), 0xEEEEEE);
        assert_eq!(media::invert_bw_ink_rgb(0xEEEEEE), 0x111111);
        assert_eq!(media::invert_bw_ink_rgb(0x0A58CA), 0x0A58CA);
        assert_eq!(media::invert_bw_ink_rgb(0xD92D20), 0xD92D20);
        assert_eq!(media::invert_bw_ink_rgb(0xFACC15), 0xFACC15);
        assert_eq!(media::invert_bw_ink_rgb(0x15803D), 0x15803D);
    }

    #[test]
    fn clipboard_pixels_repairs_windows_zero_alpha_losslessly() {
        let bgra = [
            0xFF, 0x00, 0x00, 0x00, // pure blue (B=255, G=0, R=0, A=0)
            0x00, 0x80, 0xFF, 0x00, // orange (B=0, G=128, R=255, A=0)
        ];
        let encoded = media::pixels_to_lossless_png(&bgra, 2, 1, 8, true, true);
        assert!(encoded.is_ok());
        if let Ok(png_bytes) = encoded {
            let decoded = media::normalize_clipboard_png(&png_bytes);
            assert!(decoded.is_ok());
            if let Ok((norm_bytes, w, h)) = decoded {
                assert_eq!((w, h), (2, 1));
                let decoder = png::Decoder::new(std::io::Cursor::new(&norm_bytes));
                let read_res = decoder.read_info();
                assert!(read_res.is_ok(), "failed to read normalized png");
                if let Ok(mut reader) = read_res {
                    let mut buf = vec![0u8; reader.output_buffer_size()];
                    assert!(reader.next_frame(&mut buf).is_ok());
                    assert_eq!(&buf[0..4], &[0x00, 0x00, 0xFF, 0xFF]);
                    assert_eq!(&buf[4..8], &[0xFF, 0x80, 0x00, 0xFF]);
                }
            }
        }
    }

    #[test]
    fn library_scans_xopp_and_searches_vietnamese_diacritic_insensitively() {
        let dir = tempfile::tempdir().ok();
        assert!(dir.is_some());
        if let Some(dir) = dir {
            let sub = dir.path().join("Giải tích");
            assert!(std::fs::create_dir_all(&sub).is_ok());
            let note_path = sub.join("Đạo hàm lớp 12.xopp");

            let preview_png =
                media::pixels_to_lossless_png(&[255, 0, 0, 255], 1, 1, 4, false, false);
            assert!(preview_png.is_ok());
            if let Ok(png_bytes) = preview_png {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                let xml = format!(
                    "<?xml version=\"1.0\"?><xournal><preview>{b64}</preview><page width=\"595\" height=\"842\"></page><page width=\"595\" height=\"842\"></page></xournal>"
                );
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                assert!(encoder.write_all(xml.as_bytes()).is_ok());
                if let Ok(compressed) = encoder.finish() {
                    assert!(std::fs::write(&note_path, compressed).is_ok());
                }

                let scan = library::scan_library(dir.path(), "dao ham");
                assert!(scan.is_ok());
                if let Ok(res) = scan {
                    assert_eq!(res.notes.len(), 1);
                    assert_eq!(res.notes[0].page_count, 2);
                    assert!(res.notes[0].has_preview);
                }

                let preview = library::extract_xopp_preview(&note_path);
                assert!(preview.is_ok());
                if let Ok(extracted) = preview {
                    assert_eq!(extracted, png_bytes);
                }
            }
        }
    }

    #[test]
    fn ffi_state_and_pen_slots_roundtrip() {
        let dir = tempfile::tempdir().ok();
        assert!(dir.is_some());
        if let Some(dir) = dir {
            let path = dir.path().join("ffi_state.json");
            let cpath = CString::new(path.to_string_lossy().as_ref()).ok();
            assert!(cpath.is_some());
            if let Some(cpath) = cpath {
                unsafe {
                    assert_eq!(sn_state_init(cpath.as_ptr()), SnStatus::Ok);
                    assert_eq!(sn_state_pen_slot_count(), 5);
                    let mut slot = SnPenSlot {
                        id: 0,
                        label: [0; 64],
                        kind: SnPenKind::Pen,
                        color_rgb: 0,
                        width_pt: 1.0,
                        pressure_sensitive: true,
                        line_style: SnLineStyle::Solid,
                        ink_to_shape: false,
                    };
                    assert_eq!(sn_state_get_pen_slot(0, &mut slot), SnStatus::Ok);
                    assert_eq!(slot.color_rgb, 0x111111);
                    slot.color_rgb = 0x224466;
                    slot.width_pt = 2.5;
                    assert_eq!(sn_state_set_pen_slot(0, &slot), SnStatus::Ok);
                    assert_eq!(sn_state_save(), SnStatus::Ok);
                }
                let reloaded = config::load_config(&path);
                assert_eq!(reloaded.pen_slots[0].color_rgb, 0x224466);
                assert!((reloaded.pen_slots[0].width_pt - 2.5).abs() < 1e-6);
            }
        }
    }
}

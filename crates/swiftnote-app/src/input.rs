use std::collections::BTreeMap;

use egui::{Event as EguiEvent, ImeEvent, Key as EguiKey, Modifiers, PointerButton, Pos2, Vec2};
use sdl3::event::{Event as SdlEvent, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::{MouseButton, MouseWheelDirection};
use sdl3::pen::PenAxis;
use swiftnote_core::{ActiveTool, AppSettings, InputGuard, InputMode, RawInputSample, ScreenPoint};

use crate::clipboard::{read_os_clipboard, ClipboardContent};

/// Virtual mouse ID SDL3 assigns to touch-synthesized mouse events.
pub const SDL_TOUCH_MOUSE_ID: u32 = (-1_i32) as u32;
/// Virtual mouse ID SDL3 assigns to pen-synthesized mouse events.
pub const SDL_PEN_MOUSE_ID: u32 = (-2_i32) as u32;

/// Normalized pointer/stylus sample in window logical point coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StylusSample {
    pub pos: ScreenPoint,
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub timestamp_ms: u64,
    pub is_eraser: bool,
    pub from_pen: bool,
}

/// Keyboard and tool shortcuts dispatched to `AppController`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppShortcut {
    Undo,
    Redo,
    Paste,
    Save,
    Open,
    NewNote,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    SelectPen(usize),
    ToggleEraser,
    SelectTool,
    DeleteSelected,
    ToggleSidebar,
    ToggleGallery,
}

/// Clean canvas input actions after palm, synthetic-mouse, and UI-hit isolation.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasInputEvent {
    StrokeStart(StylusSample),
    StrokeMove(StylusSample),
    StrokeEnd(StylusSample),
    PanDelta(f32, f32),
    ZoomAt { factor: f32, anchor: ScreenPoint },
    PointerHover(ScreenPoint),
    PointerLeave,
    FocusLost,
    Shortcut(AppShortcut),
}

#[derive(Debug, Clone, Copy)]
struct TouchPoint {
    pos: ScreenPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveStrokeSource {
    Pen,
    Mouse,
    SingleFinger(u64),
}

/// Translates SDL3 events into isolated canvas strokes/gestures plus `egui` input events.
pub struct SdlInputMapper {
    input_guard: InputGuard,
    viewport_logical_size: (f32, f32),
    pen_in_proximity: bool,
    pen_barrel_eraser: bool,
    pen_down_eraser: bool,
    pen_on_ui: bool,
    last_pressure: f32,
    last_tilt_x: f32,
    last_tilt_y: f32,
    last_pointer_pos: ScreenPoint,
    active_stroke: Option<ActiveStrokeSource>,
    mouse_panning: bool,
    space_held: bool,
    touches: BTreeMap<u64, TouchPoint>,
    prev_two_finger: Option<(ScreenPoint, f64)>,
    ime_composing: bool,
    last_mods: Modifiers,
    egui_events: Vec<EguiEvent>,
}

impl Default for SdlInputMapper {
    fn default() -> Self {
        Self::new()
    }
}

impl SdlInputMapper {
    pub fn new() -> Self {
        Self {
            input_guard: InputGuard::new(),
            viewport_logical_size: (1280.0, 800.0),
            pen_in_proximity: false,
            pen_barrel_eraser: false,
            pen_down_eraser: false,
            pen_on_ui: false,
            last_pressure: 0.0,
            last_tilt_x: 0.0,
            last_tilt_y: 0.0,
            last_pointer_pos: ScreenPoint::new(640.0, 400.0),
            active_stroke: None,
            mouse_panning: false,
            space_held: false,
            touches: BTreeMap::new(),
            prev_two_finger: None,
            ime_composing: false,
            last_mods: Modifiers::NONE,
            egui_events: Vec::new(),
        }
    }

    pub fn set_viewport_logical_size(&mut self, width: f32, height: f32) {
        self.viewport_logical_size = (width.max(1.0), height.max(1.0));
    }

    #[cfg(test)]
    pub fn pen_in_proximity(&self) -> bool {
        self.pen_in_proximity
    }

    pub fn take_egui_events(&mut self) -> Vec<EguiEvent> {
        std::mem::take(&mut self.egui_events)
    }

    /// Processes one SDL3 event, queuing any `egui` events and returning canvas events.
    pub fn feed_sdl_event(
        &mut self,
        event: &SdlEvent,
        settings: &AppSettings,
        active_tool: ActiveTool,
        egui_wants_pointer: bool,
        egui_wants_keyboard: bool,
    ) -> Vec<CanvasInputEvent> {
        let mut out = Vec::new();
        match event {
            SdlEvent::Window { win_event, .. } => {
                self.handle_window_event(*win_event, &mut out);
            }
            SdlEvent::PenProximityIn { .. } => {
                self.pen_in_proximity = true;
                self.touches.clear();
                self.prev_two_finger = None;
                self.mouse_panning = false;
                if matches!(
                    self.active_stroke,
                    Some(ActiveStrokeSource::Mouse | ActiveStrokeSource::SingleFinger(_))
                ) {
                    self.flush_active_stroke_as_end(&mut out);
                }
            }
            SdlEvent::PenProximityOut { .. } => {
                self.handle_pen_proximity_out(&mut out);
            }
            SdlEvent::PenAxis { axis, value, .. } => {
                self.handle_pen_axis(*axis, *value);
            }
            SdlEvent::PenButtonDown { button, .. } => {
                if *button == 1 || *button == 2 {
                    self.pen_barrel_eraser = true;
                }
            }
            SdlEvent::PenButtonUp { button, .. } => {
                if *button == 1 || *button == 2 {
                    self.pen_barrel_eraser = false;
                }
            }
            SdlEvent::PenDown {
                x,
                y,
                eraser,
                timestamp,
                ..
            } => {
                self.handle_pen_down(
                    *x,
                    *y,
                    *eraser,
                    *timestamp,
                    settings,
                    active_tool,
                    egui_wants_pointer,
                    &mut out,
                );
            }
            SdlEvent::PenMotion {
                x, y, timestamp, ..
            } => {
                self.handle_pen_motion(*x, *y, *timestamp, settings, active_tool, &mut out);
            }
            SdlEvent::PenUp {
                x,
                y,
                eraser,
                timestamp,
                ..
            } => {
                self.handle_pen_up(*x, *y, *eraser, *timestamp, settings, active_tool, &mut out);
            }
            SdlEvent::FingerDown {
                finger_id,
                x,
                y,
                pressure,
                timestamp,
                ..
            } => {
                self.handle_finger_down(
                    *finger_id,
                    *x,
                    *y,
                    *pressure,
                    *timestamp,
                    settings,
                    active_tool,
                    egui_wants_pointer,
                    &mut out,
                );
            }
            SdlEvent::FingerMotion {
                finger_id,
                x,
                y,
                pressure,
                timestamp,
                ..
            } => {
                self.handle_finger_motion(
                    *finger_id,
                    *x,
                    *y,
                    *pressure,
                    *timestamp,
                    settings,
                    active_tool,
                    &mut out,
                );
            }
            SdlEvent::FingerUp {
                finger_id,
                x,
                y,
                pressure,
                timestamp,
                ..
            } => {
                self.handle_finger_up(
                    *finger_id,
                    *x,
                    *y,
                    *pressure,
                    *timestamp,
                    settings,
                    active_tool,
                    &mut out,
                );
            }
            SdlEvent::FingerCanceled { finger_id, .. } => {
                self.touches.remove(finger_id);
                self.prev_two_finger = None;
                if self.active_stroke == Some(ActiveStrokeSource::SingleFinger(*finger_id)) {
                    self.flush_active_stroke_as_end(&mut out);
                }
            }
            SdlEvent::PinchUpdate { scale, .. } => {
                if !self.pen_in_proximity && scale.is_finite() && *scale > 0.0 && *scale != 1.0 {
                    out.push(CanvasInputEvent::ZoomAt {
                        factor: *scale,
                        anchor: self.last_pointer_pos,
                    });
                }
            }
            SdlEvent::MouseMotion {
                which,
                x,
                y,
                xrel,
                yrel,
                ..
            } => {
                self.handle_mouse_motion(*which, *x, *y, *xrel, *yrel, active_tool, &mut out);
            }
            SdlEvent::MouseButtonDown {
                which,
                mouse_btn,
                x,
                y,
                timestamp,
                ..
            } => {
                self.handle_mouse_button_down(
                    *which,
                    *mouse_btn,
                    *x,
                    *y,
                    *timestamp,
                    active_tool,
                    egui_wants_pointer,
                    &mut out,
                );
            }
            SdlEvent::MouseButtonUp {
                which,
                mouse_btn,
                x,
                y,
                timestamp,
                ..
            } => {
                self.handle_mouse_button_up(
                    *which,
                    *mouse_btn,
                    *x,
                    *y,
                    *timestamp,
                    active_tool,
                    &mut out,
                );
            }
            SdlEvent::MouseWheel {
                which,
                x,
                y,
                direction,
                mouse_x,
                mouse_y,
                ..
            } => {
                self.handle_mouse_wheel(
                    *which,
                    *x,
                    *y,
                    *direction,
                    *mouse_x,
                    *mouse_y,
                    egui_wants_pointer,
                    &mut out,
                );
            }
            SdlEvent::KeyDown {
                keycode,
                keymod,
                repeat,
                ..
            } => {
                self.handle_key_down(*keycode, *keymod, *repeat, egui_wants_keyboard, &mut out);
            }
            SdlEvent::KeyUp {
                keycode, keymod, ..
            } => {
                self.handle_key_up(*keycode, *keymod);
            }
            SdlEvent::TextEditing {
                text,
                start,
                length,
                ..
            } => {
                self.ime_composing = !text.is_empty();
                let active_range_chars = if *length > 0 && *start >= 0 {
                    Some((*start as usize)..((*start + *length) as usize))
                } else {
                    None
                };
                self.egui_events.push(EguiEvent::Ime(ImeEvent::Preedit {
                    text: text.clone(),
                    active_range_chars,
                }));
            }
            SdlEvent::TextInput { text, .. } => {
                if self.ime_composing {
                    self.ime_composing = false;
                    self.egui_events
                        .push(EguiEvent::Ime(ImeEvent::Commit(text.clone())));
                } else {
                    self.egui_events.push(EguiEvent::Text(text.clone()));
                }
            }
            _ => {}
        }
        out
    }

    fn handle_window_event(&mut self, win_event: WindowEvent, out: &mut Vec<CanvasInputEvent>) {
        match win_event {
            WindowEvent::Resized(w, h) => {
                if w > 0 && h > 0 {
                    self.set_viewport_logical_size(w as f32, h as f32);
                }
            }
            WindowEvent::MouseLeave => {
                self.egui_events.push(EguiEvent::PointerGone);
                if self.active_stroke.is_none() {
                    out.push(CanvasInputEvent::PointerLeave);
                }
            }
            WindowEvent::FocusGained => {
                self.egui_events.push(EguiEvent::WindowFocused(true));
            }
            WindowEvent::FocusLost | WindowEvent::Hidden | WindowEvent::Minimized => {
                self.egui_events.push(EguiEvent::WindowFocused(false));
                self.reset_transient_state(out);
            }
            _ => {}
        }
    }

    fn reset_transient_state(&mut self, out: &mut Vec<CanvasInputEvent>) {
        self.flush_active_stroke_as_end(out);
        self.pen_in_proximity = false;
        self.pen_barrel_eraser = false;
        self.pen_down_eraser = false;
        self.pen_on_ui = false;
        self.mouse_panning = false;
        self.space_held = false;
        self.touches.clear();
        self.prev_two_finger = None;
        self.ime_composing = false;
        self.last_mods = Modifiers::NONE;
        self.input_guard.reset();
        self.egui_events.push(EguiEvent::PointerGone);
        out.push(CanvasInputEvent::FocusLost);
    }

    fn handle_pen_proximity_out(&mut self, out: &mut Vec<CanvasInputEvent>) {
        self.flush_active_stroke_as_end(out);
        self.pen_in_proximity = false;
        self.pen_barrel_eraser = false;
        self.pen_down_eraser = false;
        self.pen_on_ui = false;
        self.input_guard.reset();
        out.push(CanvasInputEvent::PointerLeave);
    }

    fn flush_active_stroke_as_end(&mut self, out: &mut Vec<CanvasInputEvent>) {
        let Some(source) = self.active_stroke.take() else {
            return;
        };
        let from_pen = matches!(source, ActiveStrokeSource::Pen);
        let is_eraser = self.pen_down_eraser || self.pen_barrel_eraser;
        out.push(CanvasInputEvent::StrokeEnd(StylusSample {
            pos: self.last_pointer_pos,
            pressure: if self.last_pressure > 0.0 {
                self.last_pressure.clamp(0.05, 1.0)
            } else {
                0.5
            },
            tilt_x: self.last_tilt_x,
            tilt_y: self.last_tilt_y,
            timestamp_ms: 0,
            is_eraser,
            from_pen,
        }));
        self.input_guard.reset();
    }

    fn handle_pen_axis(&mut self, axis: PenAxis, value: f32) {
        if !value.is_finite() {
            return;
        }
        match axis {
            PenAxis::Pressure => {
                self.last_pressure = value.clamp(0.0, 1.0);
            }
            PenAxis::XTilt => {
                self.last_tilt_x = (value / 90.0).clamp(-1.0, 1.0);
            }
            PenAxis::YTilt => {
                self.last_tilt_y = (value / 90.0).clamp(-1.0, 1.0);
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_pen_down(
        &mut self,
        x: f32,
        y: f32,
        eraser: bool,
        timestamp_ns: u64,
        settings: &AppSettings,
        active_tool: ActiveTool,
        egui_wants_pointer: bool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        self.pen_in_proximity = true;
        self.pen_down_eraser = eraser;
        self.touches.clear();
        self.prev_two_finger = None;
        self.last_pointer_pos = ScreenPoint::new(f64::from(x), f64::from(y));

        if egui_wants_pointer {
            self.pen_on_ui = true;
            self.egui_events
                .push(EguiEvent::PointerMoved(Pos2::new(x, y)));
            self.egui_events.push(EguiEvent::PointerButton {
                pos: Pos2::new(x, y),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            });
            return;
        }

        let ts_ms = timestamp_ns / 1_000_000;
        let pressure = effective_pen_pressure(self.last_pressure, settings.input_mode);
        let raw = RawInputSample::new(
            f64::from(x),
            f64::from(y),
            f64::from(pressure),
            self.last_tilt_x,
            self.last_tilt_y,
            ts_ms,
        );
        let Some(filtered) = self.input_guard.filter_down(raw) else {
            return;
        };

        if self.last_pressure <= 0.0 {
            self.last_pressure = filtered.pressure as f32;
        }

        let is_eraser = eraser || self.pen_barrel_eraser || active_tool == ActiveTool::Eraser;
        self.active_stroke = Some(ActiveStrokeSource::Pen);
        out.push(CanvasInputEvent::StrokeStart(StylusSample {
            pos: ScreenPoint::new(filtered.x, filtered.y),
            pressure: filtered.pressure as f32,
            tilt_x: filtered.tilt_x,
            tilt_y: filtered.tilt_y,
            timestamp_ms: filtered.timestamp_ms,
            is_eraser,
            from_pen: true,
        }));
    }

    fn handle_pen_motion(
        &mut self,
        x: f32,
        y: f32,
        timestamp_ns: u64,
        settings: &AppSettings,
        active_tool: ActiveTool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        self.pen_in_proximity = true;
        self.last_pointer_pos = ScreenPoint::new(f64::from(x), f64::from(y));

        if self.pen_on_ui {
            self.egui_events
                .push(EguiEvent::PointerMoved(Pos2::new(x, y)));
            return;
        }

        if self.active_stroke != Some(ActiveStrokeSource::Pen) {
            self.egui_events
                .push(EguiEvent::PointerMoved(Pos2::new(x, y)));
            out.push(CanvasInputEvent::PointerHover(ScreenPoint::new(
                f64::from(x),
                f64::from(y),
            )));
            return;
        }

        let ts_ms = timestamp_ns / 1_000_000;
        let pressure = match settings.input_mode {
            InputMode::DigitizerPenNoPressure => 1.0,
            InputMode::DigitizerPen | InputMode::FingerCapacitivePen => {
                // Zero pressure during motion signals pen liftoff glitch; InputGuard filters it.
                self.last_pressure
            }
        };
        let raw = RawInputSample::new(
            f64::from(x),
            f64::from(y),
            f64::from(pressure),
            self.last_tilt_x,
            self.last_tilt_y,
            ts_ms,
        );
        let Some(filtered) = self.input_guard.filter_motion(raw) else {
            return;
        };

        let is_eraser =
            self.pen_down_eraser || self.pen_barrel_eraser || active_tool == ActiveTool::Eraser;
        out.push(CanvasInputEvent::StrokeMove(StylusSample {
            pos: ScreenPoint::new(filtered.x, filtered.y),
            pressure: filtered.pressure as f32,
            tilt_x: filtered.tilt_x,
            tilt_y: filtered.tilt_y,
            timestamp_ms: filtered.timestamp_ms,
            is_eraser,
            from_pen: true,
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_pen_up(
        &mut self,
        x: f32,
        y: f32,
        eraser: bool,
        timestamp_ns: u64,
        settings: &AppSettings,
        active_tool: ActiveTool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        self.last_pointer_pos = ScreenPoint::new(f64::from(x), f64::from(y));

        if self.pen_on_ui {
            self.pen_on_ui = false;
            self.egui_events
                .push(EguiEvent::PointerMoved(Pos2::new(x, y)));
            self.egui_events.push(EguiEvent::PointerButton {
                pos: Pos2::new(x, y),
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            });
            return;
        }

        if self.active_stroke != Some(ActiveStrokeSource::Pen) {
            return;
        }
        self.active_stroke = None;

        let ts_ms = timestamp_ns / 1_000_000;
        let pressure = effective_pen_pressure(self.last_pressure, settings.input_mode);
        let is_eraser = eraser
            || self.pen_down_eraser
            || self.pen_barrel_eraser
            || active_tool == ActiveTool::Eraser;
        self.pen_down_eraser = false;
        self.last_pressure = 0.0;
        self.input_guard.reset();

        out.push(CanvasInputEvent::StrokeEnd(StylusSample {
            pos: ScreenPoint::new(f64::from(x), f64::from(y)),
            pressure,
            tilt_x: self.last_tilt_x,
            tilt_y: self.last_tilt_y,
            timestamp_ms: ts_ms,
            is_eraser,
            from_pen: true,
        }));
    }

    fn normalized_touch_to_screen(&self, norm_x: f32, norm_y: f32) -> ScreenPoint {
        ScreenPoint::new(
            f64::from(norm_x * self.viewport_logical_size.0),
            f64::from(norm_y * self.viewport_logical_size.1),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_finger_down(
        &mut self,
        finger_id: u64,
        norm_x: f32,
        norm_y: f32,
        pressure: f32,
        timestamp_ns: u64,
        settings: &AppSettings,
        active_tool: ActiveTool,
        egui_wants_pointer: bool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        if self.pen_in_proximity {
            return;
        }

        let pos = self.normalized_touch_to_screen(norm_x, norm_y);
        self.touches.insert(finger_id, TouchPoint { pos });

        if self.touches.len() >= 2 {
            if matches!(
                self.active_stroke,
                Some(ActiveStrokeSource::SingleFinger(_))
            ) {
                self.flush_active_stroke_as_end(out);
            }
            self.prev_two_finger = compute_two_finger_state(&self.touches);
            return;
        }

        if egui_wants_pointer {
            return;
        }

        if settings.input_mode == InputMode::FingerCapacitivePen {
            let ts_ms = timestamp_ns / 1_000_000;
            let p = if pressure.is_finite() && pressure > 0.0 {
                pressure.clamp(0.1, 1.0)
            } else {
                0.65
            };
            let raw = RawInputSample::new(pos.x, pos.y, f64::from(p), 0.0, 0.0, ts_ms);
            if let Some(filtered) = self.input_guard.filter_down(raw) {
                self.active_stroke = Some(ActiveStrokeSource::SingleFinger(finger_id));
                self.last_pointer_pos = pos;
                out.push(CanvasInputEvent::StrokeStart(StylusSample {
                    pos: ScreenPoint::new(filtered.x, filtered.y),
                    pressure: filtered.pressure as f32,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    timestamp_ms: filtered.timestamp_ms,
                    is_eraser: active_tool == ActiveTool::Eraser,
                    from_pen: false,
                }));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_finger_motion(
        &mut self,
        finger_id: u64,
        norm_x: f32,
        norm_y: f32,
        pressure: f32,
        timestamp_ns: u64,
        settings: &AppSettings,
        active_tool: ActiveTool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        if self.pen_in_proximity {
            return;
        }

        let pos = self.normalized_touch_to_screen(norm_x, norm_y);
        self.touches.insert(finger_id, TouchPoint { pos });

        if self.touches.len() == 2 {
            if let Some((curr_center, curr_dist)) = compute_two_finger_state(&self.touches) {
                if let Some((prev_center, prev_dist)) = self.prev_two_finger {
                    let dx = (curr_center.x - prev_center.x) as f32;
                    let dy = (curr_center.y - prev_center.y) as f32;
                    if dx.abs() > 0.01 || dy.abs() > 0.01 {
                        out.push(CanvasInputEvent::PanDelta(dx, dy));
                    }
                    if prev_dist > 8.0 && curr_dist > 8.0 {
                        let factor = (curr_dist / prev_dist) as f32;
                        if (factor - 1.0).abs() > 0.001 {
                            out.push(CanvasInputEvent::ZoomAt {
                                factor,
                                anchor: curr_center,
                            });
                        }
                    }
                }
                self.prev_two_finger = Some((curr_center, curr_dist));
            }
            return;
        }

        if settings.input_mode == InputMode::FingerCapacitivePen
            && self.active_stroke == Some(ActiveStrokeSource::SingleFinger(finger_id))
        {
            let ts_ms = timestamp_ns / 1_000_000;
            let p = if pressure.is_finite() && pressure > 0.0 {
                pressure.clamp(0.1, 1.0)
            } else {
                0.65
            };
            let raw = RawInputSample::new(pos.x, pos.y, f64::from(p), 0.0, 0.0, ts_ms);
            if let Some(filtered) = self.input_guard.filter_motion(raw) {
                self.last_pointer_pos = pos;
                out.push(CanvasInputEvent::StrokeMove(StylusSample {
                    pos: ScreenPoint::new(filtered.x, filtered.y),
                    pressure: filtered.pressure as f32,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    timestamp_ms: filtered.timestamp_ms,
                    is_eraser: active_tool == ActiveTool::Eraser,
                    from_pen: false,
                }));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_finger_up(
        &mut self,
        finger_id: u64,
        norm_x: f32,
        norm_y: f32,
        pressure: f32,
        timestamp_ns: u64,
        _settings: &AppSettings,
        active_tool: ActiveTool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        self.touches.remove(&finger_id);
        if self.touches.len() < 2 {
            self.prev_two_finger = None;
        }
        if self.pen_in_proximity {
            return;
        }

        if self.active_stroke == Some(ActiveStrokeSource::SingleFinger(finger_id)) {
            self.active_stroke = None;
            let pos = self.normalized_touch_to_screen(norm_x, norm_y);
            let p = if pressure.is_finite() && pressure > 0.0 {
                pressure.clamp(0.1, 1.0)
            } else {
                0.65
            };
            self.input_guard.reset();
            out.push(CanvasInputEvent::StrokeEnd(StylusSample {
                pos,
                pressure: p,
                tilt_x: 0.0,
                tilt_y: 0.0,
                timestamp_ms: timestamp_ns / 1_000_000,
                is_eraser: active_tool == ActiveTool::Eraser,
                from_pen: false,
            }));
        }
    }

    fn is_synthetic_mouse(which: u32) -> bool {
        which == SDL_TOUCH_MOUSE_ID || which == SDL_PEN_MOUSE_ID
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mouse_motion(
        &mut self,
        which: u32,
        x: f32,
        y: f32,
        xrel: f32,
        yrel: f32,
        active_tool: ActiveTool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        if Self::is_synthetic_mouse(which) || self.pen_in_proximity {
            return;
        }
        self.last_pointer_pos = ScreenPoint::new(f64::from(x), f64::from(y));

        if self.mouse_panning {
            out.push(CanvasInputEvent::PanDelta(xrel, yrel));
            return;
        }

        if self.active_stroke == Some(ActiveStrokeSource::Mouse) {
            let sample = StylusSample {
                pos: ScreenPoint::new(f64::from(x), f64::from(y)),
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                timestamp_ms: 0,
                is_eraser: active_tool == ActiveTool::Eraser,
                from_pen: false,
            };
            out.push(CanvasInputEvent::StrokeMove(sample));
            return;
        }

        self.egui_events
            .push(EguiEvent::PointerMoved(Pos2::new(x, y)));
        out.push(CanvasInputEvent::PointerHover(ScreenPoint::new(
            f64::from(x),
            f64::from(y),
        )));
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mouse_button_down(
        &mut self,
        which: u32,
        btn: MouseButton,
        x: f32,
        y: f32,
        timestamp_ns: u64,
        active_tool: ActiveTool,
        egui_wants_pointer: bool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        if Self::is_synthetic_mouse(which) || self.pen_in_proximity {
            return;
        }

        self.last_pointer_pos = ScreenPoint::new(f64::from(x), f64::from(y));
        if let Some(egui_btn) = map_mouse_button(btn) {
            self.egui_events.push(EguiEvent::PointerButton {
                pos: Pos2::new(x, y),
                button: egui_btn,
                pressed: true,
                modifiers: Modifiers::NONE,
            });
        }

        if egui_wants_pointer {
            return;
        }

        match btn {
            MouseButton::Middle => {
                self.mouse_panning = true;
            }
            MouseButton::Left if self.space_held => {
                self.mouse_panning = true;
            }
            MouseButton::Left => {
                self.active_stroke = Some(ActiveStrokeSource::Mouse);
                out.push(CanvasInputEvent::StrokeStart(StylusSample {
                    pos: ScreenPoint::new(f64::from(x), f64::from(y)),
                    pressure: 1.0,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    timestamp_ms: timestamp_ns / 1_000_000,
                    is_eraser: active_tool == ActiveTool::Eraser,
                    from_pen: false,
                }));
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mouse_button_up(
        &mut self,
        which: u32,
        btn: MouseButton,
        x: f32,
        y: f32,
        timestamp_ns: u64,
        active_tool: ActiveTool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        if Self::is_synthetic_mouse(which) || self.pen_in_proximity {
            return;
        }

        self.last_pointer_pos = ScreenPoint::new(f64::from(x), f64::from(y));
        if let Some(egui_btn) = map_mouse_button(btn) {
            self.egui_events.push(EguiEvent::PointerButton {
                pos: Pos2::new(x, y),
                button: egui_btn,
                pressed: false,
                modifiers: Modifiers::NONE,
            });
        }

        if matches!(btn, MouseButton::Middle | MouseButton::Left) {
            self.mouse_panning = false;
        }

        if btn == MouseButton::Left && self.active_stroke == Some(ActiveStrokeSource::Mouse) {
            self.active_stroke = None;
            out.push(CanvasInputEvent::StrokeEnd(StylusSample {
                pos: ScreenPoint::new(f64::from(x), f64::from(y)),
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                timestamp_ms: timestamp_ns / 1_000_000,
                is_eraser: active_tool == ActiveTool::Eraser,
                from_pen: false,
            }));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mouse_wheel(
        &mut self,
        which: u32,
        x: f32,
        y: f32,
        direction: MouseWheelDirection,
        mouse_x: f32,
        mouse_y: f32,
        egui_wants_pointer: bool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        if Self::is_synthetic_mouse(which) {
            return;
        }

        let sign = if direction == MouseWheelDirection::Flipped {
            -1.0
        } else {
            1.0
        };
        let dx = x * sign;
        let dy = y * sign;
        let mods = self.last_mods;

        if egui_wants_pointer {
            self.egui_events.push(EguiEvent::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: Vec2::new(dx, dy),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: mods,
            });
            return;
        }

        if mods.ctrl || mods.command {
            let factor = 1.12_f32.powf(dy);
            if factor.is_finite() && (factor - 1.0).abs() > 0.001 {
                out.push(CanvasInputEvent::ZoomAt {
                    factor,
                    anchor: ScreenPoint::new(f64::from(mouse_x), f64::from(mouse_y)),
                });
            }
        } else if mods.shift {
            let step = 56.0;
            let pan_x = if dx.abs() > 0.01 { dx } else { dy } * step;
            out.push(CanvasInputEvent::PanDelta(pan_x, 0.0));
        } else {
            let step = 56.0;
            out.push(CanvasInputEvent::PanDelta(dx * step, dy * step));
        }
    }

    fn handle_key_down(
        &mut self,
        keycode: Option<Keycode>,
        keymod: Mod,
        repeat: bool,
        egui_wants_keyboard: bool,
        out: &mut Vec<CanvasInputEvent>,
    ) {
        let mods = sdl_mod_to_egui(keymod);
        self.last_mods = mods;
        self.egui_events.push(EguiEvent::ModifiersChanged(mods));

        let Some(code) = keycode else {
            return;
        };

        if let Some(egui_key) = map_keycode_to_egui(code) {
            self.egui_events.push(EguiEvent::Key {
                key: egui_key,
                physical_key: Some(egui_key),
                pressed: true,
                repeat,
                modifiers: mods,
            });
        }

        let ctrl_only = (mods.ctrl || mods.command) && !mods.alt;

        if egui_wants_keyboard {
            if ctrl_only && !repeat {
                match code {
                    Keycode::C => self.egui_events.push(EguiEvent::Copy),
                    Keycode::X => self.egui_events.push(EguiEvent::Cut),
                    Keycode::V => {
                        if let Ok(ClipboardContent::Text(text)) = read_os_clipboard() {
                            self.egui_events.push(EguiEvent::Paste(text));
                        }
                    }
                    _ => {}
                }
            }
            return;
        }

        if code == Keycode::Space && !mods.ctrl && !mods.alt {
            self.space_held = true;
            return;
        }

        if repeat {
            return;
        }

        if ctrl_only {
            let shortcut = match code {
                Keycode::Z if mods.shift => Some(AppShortcut::Redo),
                Keycode::Z => Some(AppShortcut::Undo),
                Keycode::Y => Some(AppShortcut::Redo),
                Keycode::V => Some(AppShortcut::Paste),
                Keycode::S => Some(AppShortcut::Save),
                Keycode::O => Some(AppShortcut::Open),
                Keycode::N => Some(AppShortcut::NewNote),
                Keycode::Plus | Keycode::Equals | Keycode::KpPlus => Some(AppShortcut::ZoomIn),
                Keycode::Minus | Keycode::KpMinus => Some(AppShortcut::ZoomOut),
                Keycode::_0 | Keycode::Kp0 => Some(AppShortcut::ZoomReset),
                _ => None,
            };
            if let Some(sc) = shortcut {
                out.push(CanvasInputEvent::Shortcut(sc));
            }
            return;
        }

        if !mods.ctrl && !mods.alt {
            let shortcut = match code {
                Keycode::_1 => Some(AppShortcut::SelectPen(0)),
                Keycode::_2 => Some(AppShortcut::SelectPen(1)),
                Keycode::_3 => Some(AppShortcut::SelectPen(2)),
                Keycode::_4 => Some(AppShortcut::SelectPen(3)),
                Keycode::_5 => Some(AppShortcut::SelectPen(4)),
                Keycode::E => Some(AppShortcut::ToggleEraser),
                Keycode::V => Some(AppShortcut::SelectTool),
                Keycode::G => Some(AppShortcut::ToggleGallery),
                Keycode::Tab => Some(AppShortcut::ToggleSidebar),
                Keycode::Delete | Keycode::Backspace => Some(AppShortcut::DeleteSelected),
                _ => None,
            };
            if let Some(sc) = shortcut {
                out.push(CanvasInputEvent::Shortcut(sc));
            }
        }
    }

    fn handle_key_up(&mut self, keycode: Option<Keycode>, keymod: Mod) {
        let mods = sdl_mod_to_egui(keymod);
        self.last_mods = mods;
        self.egui_events.push(EguiEvent::ModifiersChanged(mods));

        if let Some(code) = keycode {
            if code == Keycode::Space {
                self.space_held = false;
                if self.active_stroke.is_none() {
                    self.mouse_panning = false;
                }
            }
            if let Some(egui_key) = map_keycode_to_egui(code) {
                self.egui_events.push(EguiEvent::Key {
                    key: egui_key,
                    physical_key: Some(egui_key),
                    pressed: false,
                    repeat: false,
                    modifiers: mods,
                });
            }
        }
    }
}

fn effective_pen_pressure(last_pressure: f32, mode: InputMode) -> f32 {
    match mode {
        InputMode::DigitizerPenNoPressure => 1.0,
        InputMode::DigitizerPen | InputMode::FingerCapacitivePen => {
            if last_pressure > 0.0 {
                last_pressure.clamp(0.05, 1.0)
            } else {
                0.5
            }
        }
    }
}

fn compute_two_finger_state(touches: &BTreeMap<u64, TouchPoint>) -> Option<(ScreenPoint, f64)> {
    let mut iter = touches.values();
    let a = iter.next()?.pos;
    let b = iter.next()?.pos;
    let center = ScreenPoint::new(0.5 * (a.x + b.x), 0.5 * (a.y + b.y));
    let dist = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    Some((center, dist))
}

fn map_mouse_button(btn: MouseButton) -> Option<PointerButton> {
    match btn {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::X1 => Some(PointerButton::Extra1),
        MouseButton::X2 => Some(PointerButton::Extra2),
        MouseButton::Unknown => None,
    }
}

fn sdl_mod_to_egui(keymod: Mod) -> Modifiers {
    let shift = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
    let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
    let alt = keymod.intersects(Mod::LALTMOD | Mod::RALTMOD);
    let mac_cmd = cfg!(target_os = "macos") && keymod.intersects(Mod::LGUIMOD | Mod::RGUIMOD);
    let command = if cfg!(target_os = "macos") {
        mac_cmd
    } else {
        ctrl
    };
    Modifiers {
        alt,
        ctrl,
        shift,
        mac_cmd,
        command,
    }
}

fn map_keycode_to_egui(code: Keycode) -> Option<EguiKey> {
    match code {
        Keycode::Down => Some(EguiKey::ArrowDown),
        Keycode::Left => Some(EguiKey::ArrowLeft),
        Keycode::Right => Some(EguiKey::ArrowRight),
        Keycode::Up => Some(EguiKey::ArrowUp),
        Keycode::Escape => Some(EguiKey::Escape),
        Keycode::Tab => Some(EguiKey::Tab),
        Keycode::Backspace => Some(EguiKey::Backspace),
        Keycode::Return | Keycode::KpEnter => Some(EguiKey::Enter),
        Keycode::Space => Some(EguiKey::Space),
        Keycode::Insert => Some(EguiKey::Insert),
        Keycode::Delete => Some(EguiKey::Delete),
        Keycode::Home => Some(EguiKey::Home),
        Keycode::End => Some(EguiKey::End),
        Keycode::PageUp => Some(EguiKey::PageUp),
        Keycode::PageDown => Some(EguiKey::PageDown),
        Keycode::Minus | Keycode::KpMinus => Some(EguiKey::Minus),
        Keycode::Plus | Keycode::KpPlus => Some(EguiKey::Plus),
        Keycode::Equals => Some(EguiKey::Equals),
        Keycode::_0 | Keycode::Kp0 => Some(EguiKey::Num0),
        Keycode::_1 | Keycode::Kp1 => Some(EguiKey::Num1),
        Keycode::_2 | Keycode::Kp2 => Some(EguiKey::Num2),
        Keycode::_3 | Keycode::Kp3 => Some(EguiKey::Num3),
        Keycode::_4 | Keycode::Kp4 => Some(EguiKey::Num4),
        Keycode::_5 | Keycode::Kp5 => Some(EguiKey::Num5),
        Keycode::_6 | Keycode::Kp6 => Some(EguiKey::Num6),
        Keycode::_7 | Keycode::Kp7 => Some(EguiKey::Num7),
        Keycode::_8 | Keycode::Kp8 => Some(EguiKey::Num8),
        Keycode::_9 | Keycode::Kp9 => Some(EguiKey::Num9),
        Keycode::A => Some(EguiKey::A),
        Keycode::C => Some(EguiKey::C),
        Keycode::E => Some(EguiKey::E),
        Keycode::G => Some(EguiKey::G),
        Keycode::N => Some(EguiKey::N),
        Keycode::O => Some(EguiKey::O),
        Keycode::P => Some(EguiKey::P),
        Keycode::S => Some(EguiKey::S),
        Keycode::V => Some(EguiKey::V),
        Keycode::X => Some(EguiKey::X),
        Keycode::Y => Some(EguiKey::Y),
        Keycode::Z => Some(EguiKey::Z),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::mouse::MouseState;

    fn default_settings() -> AppSettings {
        AppSettings::default()
    }

    #[test]
    fn stylus_proximity_rejects_palm_finger_and_synthesized_mouse_events() {
        let mut mapper = SdlInputMapper::new();
        let settings = default_settings();

        // Enter stylus proximity.
        let out = mapper.feed_sdl_event(
            &SdlEvent::PenProximityIn {
                timestamp: 10_000_000,
                which: 1,
                window: 1,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(out.is_empty());
        assert!(mapper.pen_in_proximity());

        // Palm / finger events while stylus is in proximity must produce zero canvas events.
        let finger = mapper.feed_sdl_event(
            &SdlEvent::FingerDown {
                timestamp: 11_000_000,
                touch_id: 1,
                finger_id: 7,
                x: 0.4,
                y: 0.5,
                dx: 0.0,
                dy: 0.0,
                pressure: 0.9,
                window_id: 1,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(finger.is_empty());

        // Synthesized mouse down / motion while stylus is near must produce zero canvas & egui events.
        let synth_down = mapper.feed_sdl_event(
            &SdlEvent::MouseButtonDown {
                timestamp: 12_000_000,
                window_id: 1,
                which: SDL_PEN_MOUSE_ID,
                mouse_btn: MouseButton::Left,
                clicks: 1,
                x: 120.0,
                y: 220.0,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(synth_down.is_empty());

        let synth_touch_move = mapper.feed_sdl_event(
            &SdlEvent::MouseMotion {
                timestamp: 13_000_000,
                window_id: 1,
                which: SDL_TOUCH_MOUSE_ID,
                mousestate: MouseState::from_sdl_state(1),
                x: 125.0,
                y: 225.0,
                xrel: 5.0,
                yrel: 5.0,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(synth_touch_move.is_empty());
        assert!(mapper.take_egui_events().is_empty());
    }

    #[test]
    fn hardware_eraser_and_barrel_button_route_to_eraser_stream() {
        let mut mapper = SdlInputMapper::new();
        let settings = default_settings();

        // Hardware inverted eraser tip.
        mapper.feed_sdl_event(
            &SdlEvent::PenAxis {
                timestamp: 1_000_000,
                which: 1,
                window: 1,
                x: 50.0,
                y: 60.0,
                axis: PenAxis::Pressure,
                value: 0.75,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        let start_hw = mapper.feed_sdl_event(
            &SdlEvent::PenDown {
                timestamp: 2_000_000,
                which: 1,
                window: 1,
                x: 50.0,
                y: 60.0,
                eraser: true,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(matches!(
            start_hw.as_slice(),
            [CanvasInputEvent::StrokeStart(StylusSample {
                is_eraser: true,
                from_pen: true,
                ..
            })]
        ));
        mapper.feed_sdl_event(
            &SdlEvent::PenUp {
                timestamp: 3_000_000,
                which: 1,
                window: 1,
                x: 50.0,
                y: 60.0,
                eraser: true,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );

        // Barrel button held with nib down must also route to eraser even when eraser=false.
        mapper.feed_sdl_event(
            &SdlEvent::PenButtonDown {
                timestamp: 4_000_000,
                which: 1,
                window: 1,
                x: 80.0,
                y: 90.0,
                button: 1,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        let start_barrel = mapper.feed_sdl_event(
            &SdlEvent::PenDown {
                timestamp: 5_000_000,
                which: 1,
                window: 1,
                x: 80.0,
                y: 90.0,
                eraser: false,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(matches!(
            start_barrel.as_slice(),
            [CanvasInputEvent::StrokeStart(StylusSample {
                is_eraser: true,
                from_pen: true,
                ..
            })]
        ));
    }

    #[test]
    fn focus_loss_and_proximity_out_flush_active_stroke_without_stuck_state() {
        let mut mapper = SdlInputMapper::new();
        let settings = default_settings();

        let down = mapper.feed_sdl_event(
            &SdlEvent::PenDown {
                timestamp: 10_000_000,
                which: 1,
                window: 1,
                x: 100.0,
                y: 120.0,
                eraser: false,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert_eq!(down.len(), 1);

        // Window focus loss mid-stroke must emit StrokeEnd + FocusLost cleanly.
        let lost = mapper.feed_sdl_event(
            &SdlEvent::Window {
                timestamp: 15_000_000,
                window_id: 1,
                win_event: WindowEvent::FocusLost,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(matches!(
            lost.as_slice(),
            [CanvasInputEvent::StrokeEnd(_), CanvasInputEvent::FocusLost]
        ));
        assert!(!mapper.pen_in_proximity());

        // Start another stroke and leave proximity without PenUp.
        mapper.feed_sdl_event(
            &SdlEvent::PenDown {
                timestamp: 20_000_000,
                which: 1,
                window: 1,
                x: 110.0,
                y: 130.0,
                eraser: false,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        let prox_out = mapper.feed_sdl_event(
            &SdlEvent::PenProximityOut {
                timestamp: 25_000_000,
                which: 1,
                window: 1,
            },
            &settings,
            ActiveTool::Pen(0),
            false,
            false,
        );
        assert!(matches!(
            prox_out.as_slice(),
            [
                CanvasInputEvent::StrokeEnd(_),
                CanvasInputEvent::PointerLeave
            ]
        ));
        assert!(!mapper.pen_in_proximity());
    }
}

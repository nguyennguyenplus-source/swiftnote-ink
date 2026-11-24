use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use rfd::FileDialog;
use swiftnote_core::{
    erase_at, load_swnote, now_unix_ms, save_swnote_atomic, ActiveTool, AppSettings, BrushStyle,
    CanvasPoint, ContourMesh, DirtyRegion, Document, EraserMode, ImageAsset, ImagePlacement,
    InputMode, LibraryEntry, NoteLibrary, PageBackground, PageId, PagePoint, PaperPolarity,
    PaperStandard, PaperTemplate, PdfAssetMeta, RawInputSample, Rect, ScreenPoint,
    StabilizerPreset, Stroke, StrokeId, StrokePoint, StrokeStabilizer, StrokeTessellator,
    ToolState, UiTheme, UndoCommand, UndoStack, ViewportTransform,
};
use swiftnote_gpu::{GpuError, PdfWorker, UnifiedRenderer};

use crate::clipboard::{read_os_clipboard, ClipboardContent};
use crate::input::{AppShortcut, CanvasInputEvent, StylusSample};

const SETTINGS_FILE_NAME: &str = "settings.json";
const MAX_PASTE_WIDTH_RATIO: f64 = 0.75;
const MAX_PASTE_HEIGHT_RATIO: f64 = 0.75;
const HANDLE_HIT_RADIUS_PX: f64 = 12.0;
const MIN_IMAGE_EXTENT_PT: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarPopover {
    None,
    Pen,
    Eraser,
    PaperAndSettings,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone)]
pub struct PdfPasswordModalState {
    pub file_name: String,
    pub raw_bytes: Vec<u8>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToastNotice {
    pub message: String,
    pub is_error: bool,
    pub expires_at: Instant,
}

/// Unidirectional UI actions emitted by the pure view and executed sequentially after `egui::Context::run`.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    ToggleGallery,
    TogglePagesSidebar,
    TogglePopover(ToolbarPopover),
    ClosePopovers,
    NewNote,
    OpenLibraryNote(String),
    DeleteLibraryNote(String),
    RenameDocument(String),
    ImportSwnote,
    ExportSwnoteCopy,
    ImportPdf,
    InsertImageFromDisk,
    PasteFromClipboard,
    SubmitPdfPassword(String),
    CancelPdfPassword,
    SelectPenSlot(usize),
    UpdateActivePenBrush(BrushStyle),
    SelectEraser,
    SetEraserMode(EraserMode),
    SetBackToPen(bool),
    SelectImageTool,
    DeleteSelectedImage,
    Undo,
    Redo,
    JumpToPage(PageId),
    AddPage,
    DeletePage(PageId),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    SetPageTemplate(PaperTemplate),
    SetPagePolarity(PaperPolarity),
    SetPageStandard(PaperStandard),
    SetUiTheme(UiTheme),
    SetInputMode(InputMode),
    SetStabilizerPreset(StabilizerPreset),
    SetAutoInvertMonochromeInk(bool),
    DismissToast,
}

struct ActiveInkSession {
    page_id: PageId,
    page_origin_canvas: CanvasPoint,
    page_w_pt: f64,
    page_h_pt: f64,
    style: BrushStyle,
    points: Vec<StrokePoint>,
    mesh: ContourMesh,
    last_sample: RawInputSample,
    is_hardware_eraser: bool,
}

struct ActiveEraseSession {
    page_id: PageId,
    page_origin_canvas: CanvasPoint,
    prev_pt: Option<PagePoint>,
    removed: Vec<Stroke>,
    added: Vec<Stroke>,
    added_ids: HashSet<StrokeId>,
    dirty: Rect,
}

struct ActiveImageDrag {
    page_id: PageId,
    page_origin_canvas: CanvasPoint,
    placement_id: u64,
    handle: Option<CornerHandle>,
    start_pointer_page: PagePoint,
    orig_bounds: Rect,
}

pub struct AppController {
    pub doc: Document,
    library: Option<NoteLibrary>,
    recent_notes_cache: Vec<LibraryEntry>,
    pub settings: AppSettings,
    pub viewport: ViewportTransform,
    pub tool_state: ToolState,
    pub undo_stack: UndoStack,
    stabilizer: StrokeStabilizer,
    tessellator: StrokeTessellator,
    pdf_worker: PdfWorker,

    active_ink: Option<ActiveInkSession>,
    active_erase: Option<ActiveEraseSession>,
    active_image_drag: Option<ActiveImageDrag>,
    pub selected_image: Option<(PageId, u64)>,
    pub hover_cursor: Option<ScreenPoint>,

    pub gallery_open: bool,
    pub pages_sidebar_open: bool,
    pub open_popover: ToolbarPopover,
    pub pdf_password_modal: Option<PdfPasswordModalState>,
    pub toast: Option<ToastNotice>,

    dirty_since: Option<Instant>,
    last_saved_at: Option<Instant>,
    centered_initial_viewport: bool,
    last_logical_w: f64,
}

impl AppController {
    /// Creates an `AppController` backed by `library` (or purely in-memory if `None`).
    pub fn new_with_library(library: Option<NoteLibrary>) -> Self {
        let mut settings = library
            .as_ref()
            .and_then(load_settings_from_library)
            .unwrap_or_default();

        let (doc, recent_notes_cache) = match library {
            Some(ref lib) => {
                let recents = lib.list_recent_notes();
                if let Some(first) = recents.first() {
                    if let Ok(loaded) = lib.open_note(&first.note_id) {
                        (loaded, recents)
                    } else {
                        (
                            Document::new_untitled(
                                format!("note_{}", now_unix_ms()),
                                "Untitled Note",
                                now_unix_ms(),
                            ),
                            recents,
                        )
                    }
                } else {
                    (
                        Document::new_untitled(
                            format!("note_{}", now_unix_ms()),
                            "Untitled Note",
                            now_unix_ms(),
                        ),
                        recents,
                    )
                }
            }
            None => (
                Document::new_untitled("note_local", "Untitled Note", now_unix_ms()),
                Vec::new(),
            ),
        };

        let mut tool_state = ToolState::default();
        tool_state.back_to_pen_after_erase = settings.back_to_pen_after_erase;
        settings.autosave_interval_secs = settings.autosave_interval_secs.max(1);

        let stabilizer = StrokeStabilizer::new(settings.stabilizer_preset);
        let mut controller = Self {
            doc,
            library,
            recent_notes_cache,
            settings,
            viewport: ViewportTransform::default(),
            tool_state,
            undo_stack: UndoStack::new(),
            stabilizer,
            tessellator: StrokeTessellator::new(),
            pdf_worker: PdfWorker::spawn(),
            active_ink: None,
            active_erase: None,
            active_image_drag: None,
            selected_image: None,
            hover_cursor: None,
            gallery_open: false,
            pages_sidebar_open: false,
            open_popover: ToolbarPopover::None,
            pdf_password_modal: None,
            toast: None,
            dirty_since: None,
            last_saved_at: None,
            centered_initial_viewport: false,
            last_logical_w: 1280.0,
        };

        if let Some(ref mut lib) = controller.library {
            if controller.recent_notes_cache.is_empty() {
                if lib.save_note(&controller.doc).is_ok() {
                    controller.recent_notes_cache = lib.list_recent_notes();
                    controller.last_saved_at = Some(Instant::now());
                }
            } else {
                controller.last_saved_at = Some(Instant::now());
            }
        }
        controller.register_doc_pdfs_with_worker();

        controller
    }

    pub fn update_viewport_logical_width(&mut self, logical_w: f64) {
        if logical_w.is_finite() && logical_w > 0.0 {
            self.last_logical_w = logical_w;
        }
    }

    /// Ensures initial viewport horizontally centers page 1 cleanly on first window layout.
    pub fn ensure_initial_viewport_centered(
        &mut self,
        viewport_px: (u32, u32),
        display_scale: f32,
    ) {
        if viewport_px.0 == 0 || viewport_px.1 == 0 {
            return;
        }
        let scale = f64::from(display_scale.max(1.0));
        let logical_w = f64::from(viewport_px.0) / scale;
        self.last_logical_w = logical_w;
        if self.centered_initial_viewport {
            return;
        }
        self.viewport.pan_x = logical_w * 0.5;
        self.viewport.pan_y = 64.0;
        self.centered_initial_viewport = true;
    }

    pub fn recent_notes(&self) -> &[LibraryEntry] {
        &self.recent_notes_cache
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub fn last_saved_elapsed_secs(&self) -> Option<u64> {
        self.last_saved_at.map(|t| t.elapsed().as_secs())
    }

    pub fn has_active_stroke(&self) -> bool {
        self.active_ink.is_some() || self.active_erase.is_some() || self.active_image_drag.is_some()
    }

    /// Returns the 0-based page index closest to the vertical center of the viewport.
    pub fn active_page_index(&self, viewport_px: (u32, u32), display_scale: f32) -> usize {
        if self.doc.pages.is_empty() {
            return 0;
        }
        let scale = f64::from(display_scale.max(1.0));
        let center_y_scr = (f64::from(viewport_px.1) / scale) * 0.5;
        let center_canvas = self
            .viewport
            .screen_to_canvas(ScreenPoint::new(0.0, center_y_scr));

        let mut best_idx = 0usize;
        let mut best_dist = f64::INFINITY;
        for (idx, _page) in self.doc.pages.iter().enumerate() {
            if let Some(rect) = self.doc.page_canvas_rect(idx) {
                let mid_y = 0.5 * (rect.min_y + rect.max_y);
                let dist = (center_canvas.y - mid_y).abs();
                if rect.min_y <= center_canvas.y && center_canvas.y <= rect.max_y {
                    return idx;
                }
                if dist < best_dist {
                    best_dist = dist;
                    best_idx = idx;
                }
            }
        }
        best_idx
    }

    /// Requests background rasterization of visible PDF page tiles.
    pub fn request_visible_pdf_tiles(
        &self,
        renderer: &mut UnifiedRenderer,
        viewport_px: (u32, u32),
        display_scale: f32,
    ) {
        renderer.request_visible_pdf_tiles(
            &self.doc,
            viewport_px,
            display_scale,
            self.viewport,
            &self.pdf_worker,
        );
    }

    fn register_doc_pdfs_with_worker(&self) {
        for pdf in self.doc.pdfs.values() {
            if !pdf.encrypted {
                let _ = self.pdf_worker.open_document(pdf.raw_bytes.clone(), None);
            }
        }
    }

    /// Drains completed background PDF tiles and runs debounced autosave.
    pub fn tick(&mut self, renderer: Option<&mut UnifiedRenderer>) -> bool {
        let mut needs_redraw = false;

        let completed = self.pdf_worker.poll_completed();
        if !completed.is_empty() {
            if let Some(r) = renderer {
                for tile in completed {
                    if let Err(err) = r.upload_pdf_tile(tile, &self.doc) {
                        self.set_toast(format!("PDF tile upload error: {err}"), true);
                    } else {
                        needs_redraw = true;
                    }
                }
            }
        }

        if let Some(dirty_at) = self.dirty_since {
            let interval = Duration::from_secs(self.settings.autosave_interval_secs.max(1));
            if dirty_at.elapsed() >= interval {
                self.persist_document_now();
                needs_redraw = true;
            }
        }

        if let Some(toast) = &self.toast {
            if Instant::now() >= toast.expires_at {
                self.toast = None;
                needs_redraw = true;
            }
        }

        needs_redraw
    }

    /// Saves pending modifications synchronously when the application exits.
    pub fn flush_autosave_on_exit(&mut self) {
        if self.dirty_since.is_some() {
            self.persist_document_now();
        }
        self.persist_settings();
    }

    /// Applies queued UI actions sequentially outside `egui::Context::run`.
    pub fn apply_actions(
        &mut self,
        actions: Vec<UiAction>,
        viewport_px: (u32, u32),
        display_scale: f32,
        mut renderer: Option<&mut UnifiedRenderer>,
    ) -> bool {
        if actions.is_empty() {
            return false;
        }
        for action in actions {
            self.apply_single_action(action, viewport_px, display_scale, renderer.as_deref_mut());
        }
        true
    }

    fn apply_single_action(
        &mut self,
        action: UiAction,
        viewport_px: (u32, u32),
        display_scale: f32,
        mut renderer: Option<&mut UnifiedRenderer>,
    ) {
        match action {
            UiAction::ToggleGallery => {
                self.gallery_open = !self.gallery_open;
                self.open_popover = ToolbarPopover::None;
            }
            UiAction::TogglePagesSidebar => {
                self.pages_sidebar_open = !self.pages_sidebar_open;
                self.open_popover = ToolbarPopover::None;
            }
            UiAction::TogglePopover(target) => {
                self.open_popover = if self.open_popover == target {
                    ToolbarPopover::None
                } else {
                    target
                };
            }
            UiAction::ClosePopovers => {
                self.open_popover = ToolbarPopover::None;
            }
            UiAction::NewNote => {
                self.create_new_note(renderer);
            }
            UiAction::OpenLibraryNote(note_id) => {
                self.open_library_note(&note_id, renderer);
            }
            UiAction::DeleteLibraryNote(note_id) => {
                self.delete_library_note(&note_id);
            }
            UiAction::RenameDocument(new_title) => {
                if self.doc.title != new_title {
                    self.doc.title = new_title;
                    self.mark_dirty();
                }
            }
            UiAction::ImportSwnote => {
                self.open_popover = ToolbarPopover::None;
                if let Some(path) = FileDialog::new()
                    .add_filter("Swift Note", &["swnote"])
                    .pick_file()
                {
                    self.import_swnote_from_path(&path, renderer);
                }
            }
            UiAction::ExportSwnoteCopy => {
                self.open_popover = ToolbarPopover::None;
                let default_name = format!("{}.swnote", sanitize_filename(&self.doc.title));
                if let Some(path) = FileDialog::new()
                    .set_file_name(default_name)
                    .add_filter("Swift Note", &["swnote"])
                    .save_file()
                {
                    let res = if let Some(ref lib) = self.library {
                        lib.export_swnote_copy(&self.doc, &path)
                    } else {
                        save_swnote_atomic(&self.doc, &path)
                    };
                    match res {
                        Ok(()) => self.set_toast("Saved copy successfully", false),
                        Err(err) => self.set_toast(format!("Failed to save copy: {err}"), true),
                    }
                }
            }
            UiAction::ImportPdf => {
                self.open_popover = ToolbarPopover::None;
                if let Some(path) = FileDialog::new()
                    .add_filter("PDF Document", &["pdf"])
                    .pick_file()
                {
                    self.import_pdf_from_path(&path, renderer);
                }
            }
            UiAction::InsertImageFromDisk => {
                self.open_popover = ToolbarPopover::None;
                if let Some(path) = FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_file()
                {
                    self.insert_image_from_path(&path, viewport_px, display_scale, renderer);
                }
            }
            UiAction::PasteFromClipboard => {
                self.open_popover = ToolbarPopover::None;
                self.paste_from_clipboard(viewport_px, display_scale, renderer);
            }
            UiAction::SubmitPdfPassword(pw) => {
                self.submit_pdf_password(pw, renderer);
            }
            UiAction::CancelPdfPassword => {
                self.pdf_password_modal = None;
            }
            UiAction::SelectPenSlot(slot) => {
                let was_active =
                    matches!(self.tool_state.active_tool, ActiveTool::Pen(s) if s == slot);
                self.tool_state.select_pen_slot(slot);
                self.selected_image = None;
                if was_active {
                    self.open_popover = if self.open_popover == ToolbarPopover::Pen {
                        ToolbarPopover::None
                    } else {
                        ToolbarPopover::Pen
                    };
                } else {
                    self.open_popover = ToolbarPopover::None;
                }
            }
            UiAction::UpdateActivePenBrush(style) => {
                *self.tool_state.active_brush_mut() = style;
                self.tool_state.active_tool = ActiveTool::Pen(self.tool_state.active_pen_slot);
            }
            UiAction::SelectEraser => {
                let was_active = matches!(self.tool_state.active_tool, ActiveTool::Eraser);
                self.tool_state.select_eraser();
                self.selected_image = None;
                if was_active {
                    self.open_popover = if self.open_popover == ToolbarPopover::Eraser {
                        ToolbarPopover::None
                    } else {
                        ToolbarPopover::Eraser
                    };
                } else {
                    self.open_popover = ToolbarPopover::None;
                }
            }
            UiAction::SetEraserMode(mode) => {
                self.tool_state.eraser_mode = mode;
                self.tool_state.select_eraser();
            }
            UiAction::SetBackToPen(enabled) => {
                self.settings.back_to_pen_after_erase = enabled;
                self.tool_state.back_to_pen_after_erase = enabled;
                self.persist_settings();
            }
            UiAction::SelectImageTool => {
                self.tool_state.active_tool = ActiveTool::Select;
                self.open_popover = ToolbarPopover::None;
            }
            UiAction::DeleteSelectedImage => {
                self.delete_selected_image(renderer);
            }
            UiAction::Undo => {
                if let Some(dirty) = self.undo_stack.undo(&mut self.doc) {
                    self.selected_image = None;
                    if let Some(r) = renderer.as_deref_mut() {
                        r.invalidate_dirty(dirty);
                    }
                    self.mark_dirty();
                }
            }
            UiAction::Redo => {
                if let Some(dirty) = self.undo_stack.redo(&mut self.doc) {
                    self.selected_image = None;
                    if let Some(r) = renderer.as_deref_mut() {
                        r.invalidate_dirty(dirty);
                    }
                    self.mark_dirty();
                }
            }
            UiAction::JumpToPage(page_id) => {
                self.jump_to_page(page_id);
            }
            UiAction::AddPage => {
                let new_id = self.doc.add_page(
                    self.doc.default_standard,
                    self.doc.default_background.clone(),
                );
                if let Some(idx) = self.doc.pages.iter().position(|p| p.id == new_id) {
                    let page = self.doc.pages[idx].clone();
                    self.undo_stack
                        .push(UndoCommand::AddPage { index: idx, page });
                }
                self.jump_to_page(new_id);
                self.mark_dirty();
            }
            UiAction::DeletePage(page_id) => {
                if self.doc.pages.len() <= 1 {
                    self.set_toast("Cannot delete the last page", true);
                    return;
                }
                if let Ok((idx, page)) = self.doc.delete_page(page_id) {
                    if self.selected_image.is_some_and(|(p, _)| p == page_id) {
                        self.selected_image = None;
                    }
                    self.undo_stack
                        .push(UndoCommand::DeletePage { index: idx, page });
                    if let Some(r) = renderer.as_deref_mut() {
                        r.invalidate_page(page_id);
                    }
                    self.mark_dirty();
                }
            }
            UiAction::ZoomIn => {
                self.zoom_centered(1.2, viewport_px, display_scale);
            }
            UiAction::ZoomOut => {
                self.zoom_centered(1.0 / 1.2, viewport_px, display_scale);
            }
            UiAction::ZoomReset => {
                let scale = f64::from(display_scale.max(1.0));
                let anchor = ScreenPoint::new(
                    0.5 * f64::from(viewport_px.0) / scale,
                    0.5 * f64::from(viewport_px.1) / scale,
                );
                self.viewport.set_zoom_at(anchor, 1.0);
                self.pdf_worker.advance_generation();
            }
            UiAction::SetPageTemplate(template) => {
                let idx = self.active_page_index(viewport_px, display_scale);
                if let Some(page) = self.doc.pages.get_mut(idx) {
                    if let PageBackground::Procedural { polarity, .. } = &page.background {
                        let old_bg = page.background.clone();
                        let new_bg = PageBackground::Procedural {
                            template,
                            polarity: *polarity,
                        };
                        let page_id = page.id;
                        page.background = new_bg.clone();
                        self.doc.default_background = new_bg.clone();
                        self.undo_stack.push(UndoCommand::ChangePageBackground {
                            page_id,
                            old_bg,
                            new_bg,
                        });
                        if let Some(r) = renderer.as_deref_mut() {
                            r.invalidate_page(page_id);
                        }
                        self.mark_dirty();
                    }
                }
            }
            UiAction::SetPagePolarity(polarity) => {
                let idx = self.active_page_index(viewport_px, display_scale);
                if let Some(page) = self.doc.pages.get_mut(idx) {
                    if let PageBackground::Procedural { template, .. } = &page.background {
                        let old_bg = page.background.clone();
                        let new_bg = PageBackground::Procedural {
                            template: *template,
                            polarity,
                        };
                        let page_id = page.id;
                        page.background = new_bg.clone();
                        self.doc.default_background = new_bg.clone();
                        self.undo_stack.push(UndoCommand::ChangePageBackground {
                            page_id,
                            old_bg,
                            new_bg,
                        });
                        if let Some(r) = renderer.as_deref_mut() {
                            r.invalidate_page(page_id);
                        }
                        self.mark_dirty();
                    }
                }
            }
            UiAction::SetPageStandard(standard) => {
                self.doc.default_standard = standard;
                let idx = self.active_page_index(viewport_px, display_scale);
                if let Some(page) = self.doc.pages.get_mut(idx) {
                    if !page.is_fixed_height() {
                        let (w, h) = standard.dimensions_pt();
                        page.width_pt = w;
                        page.height_pt = h;
                        let page_id = page.id;
                        if let Some(r) = renderer.as_deref_mut() {
                            r.invalidate_page(page_id);
                        }
                        self.mark_dirty();
                    }
                }
            }
            UiAction::SetUiTheme(theme) => {
                self.settings.ui_theme = theme;
                self.persist_settings();
            }
            UiAction::SetInputMode(mode) => {
                self.settings.input_mode = mode;
                self.persist_settings();
            }
            UiAction::SetStabilizerPreset(preset) => {
                self.settings.stabilizer_preset = preset;
                self.stabilizer.set_preset(preset);
                self.persist_settings();
            }
            UiAction::SetAutoInvertMonochromeInk(enabled) => {
                self.settings.auto_invert_monochrome_ink = enabled;
                if let Some(r) = renderer {
                    r.clear_all_tiles();
                }
                self.persist_settings();
            }
            UiAction::DismissToast => {
                self.toast = None;
            }
        }
    }

    /// Processes isolated stylus, touch, mouse, or keyboard canvas input.
    pub fn handle_canvas_event(
        &mut self,
        event: CanvasInputEvent,
        viewport_px: (u32, u32),
        display_scale: f32,
        renderer: Option<&mut UnifiedRenderer>,
    ) -> bool {
        match event {
            CanvasInputEvent::PointerHover(pos) => {
                self.hover_cursor = Some(pos);
                true
            }
            CanvasInputEvent::PointerLeave => {
                self.hover_cursor = None;
                false
            }
            CanvasInputEvent::FocusLost => {
                self.flush_in_flight_interactions(renderer);
                true
            }
            CanvasInputEvent::PanDelta(dx, dy) => {
                self.viewport.pan_by(f64::from(dx), f64::from(dy));
                self.pdf_worker.advance_generation();
                true
            }
            CanvasInputEvent::ZoomAt { factor, anchor } => {
                self.viewport
                    .set_zoom_at(anchor, self.viewport.zoom * f64::from(factor));
                self.pdf_worker.advance_generation();
                true
            }
            CanvasInputEvent::Shortcut(shortcut) => {
                self.apply_shortcut(shortcut, viewport_px, display_scale, renderer);
                true
            }
            CanvasInputEvent::StrokeStart(sample) => {
                self.open_popover = ToolbarPopover::None;
                self.hover_cursor = Some(sample.pos);
                self.handle_stroke_start(sample, renderer)
            }
            CanvasInputEvent::StrokeMove(sample) => {
                self.hover_cursor = Some(sample.pos);
                self.handle_stroke_move(sample, renderer)
            }
            CanvasInputEvent::StrokeEnd(sample) => {
                self.hover_cursor = Some(sample.pos);
                self.handle_stroke_end(sample, renderer)
            }
        }
    }

    fn apply_shortcut(
        &mut self,
        shortcut: AppShortcut,
        viewport_px: (u32, u32),
        display_scale: f32,
        renderer: Option<&mut UnifiedRenderer>,
    ) {
        let action = match shortcut {
            AppShortcut::Undo => UiAction::Undo,
            AppShortcut::Redo => UiAction::Redo,
            AppShortcut::Paste => UiAction::PasteFromClipboard,
            AppShortcut::Save => UiAction::ExportSwnoteCopy,
            AppShortcut::Open => UiAction::ImportSwnote,
            AppShortcut::NewNote => UiAction::NewNote,
            AppShortcut::ZoomIn => UiAction::ZoomIn,
            AppShortcut::ZoomOut => UiAction::ZoomOut,
            AppShortcut::ZoomReset => UiAction::ZoomReset,
            AppShortcut::SelectPen(slot) => UiAction::SelectPenSlot(slot),
            AppShortcut::ToggleEraser => UiAction::SelectEraser,
            AppShortcut::SelectTool => UiAction::SelectImageTool,
            AppShortcut::DeleteSelected => UiAction::DeleteSelectedImage,
            AppShortcut::ToggleSidebar => UiAction::TogglePagesSidebar,
            AppShortcut::ToggleGallery => UiAction::ToggleGallery,
        };
        self.apply_single_action(action, viewport_px, display_scale, renderer);
    }

    fn handle_stroke_start(
        &mut self,
        sample: StylusSample,
        mut renderer: Option<&mut UnifiedRenderer>,
    ) -> bool {
        let canvas_pt = self.viewport.screen_to_canvas(sample.pos);
        let Some((page_id, page_pt)) = self.doc.page_at_canvas_point(canvas_pt) else {
            self.selected_image = None;
            return false;
        };
        let Some(page_idx) = self.doc.pages.iter().position(|p| p.id == page_id) else {
            return false;
        };
        let Some(page_rect) = self.doc.page_canvas_rect(page_idx) else {
            return false;
        };
        let page_origin_canvas = CanvasPoint::new(page_rect.min_x, page_rect.min_y);

        if self.tool_state.active_tool == ActiveTool::Select && !sample.is_eraser {
            return self.start_image_select_or_drag(page_id, page_origin_canvas, page_pt);
        }

        if sample.is_eraser {
            if self.tool_state.active_tool != ActiveTool::Eraser {
                self.tool_state.on_hardware_eraser_down();
            }
            let mode = self.tool_state.eraser_mode;
            let mut session = ActiveEraseSession {
                page_id,
                page_origin_canvas,
                prev_pt: Some(page_pt),
                removed: Vec::new(),
                added: Vec::new(),
                added_ids: HashSet::new(),
                dirty: Rect::EMPTY,
            };
            self.run_erase_step(&mut session, page_pt, mode, renderer.as_deref_mut());
            self.active_erase = Some(session);
            return true;
        }

        let style = self.tool_state.active_brush();
        let page_w_pt = page_rect.max_x - page_rect.min_x;
        let page_h_pt = page_rect.max_y - page_rect.min_y;
        let raw = RawInputSample::new(
            page_pt.x,
            page_pt.y,
            f64::from(sample.pressure),
            sample.tilt_x,
            sample.tilt_y,
            sample.timestamp_ms,
        );

        self.stabilizer.set_preset(self.settings.stabilizer_preset);
        let mut points = Vec::with_capacity(64);
        self.stabilizer.begin(raw, style, &mut points);

        self.tessellator.reset();
        let mut mesh = ContourMesh::with_capacity(256, 512);
        self.tessellator
            .tessellate_incremental(&points, style, &mut mesh);
        if let Some(r) = renderer {
            r.update_active_stroke(page_id, &mesh, style);
        }

        self.active_ink = Some(ActiveInkSession {
            page_id,
            page_origin_canvas,
            page_w_pt,
            page_h_pt,
            style,
            points,
            mesh,
            last_sample: raw,
            is_hardware_eraser: false,
        });
        true
    }

    fn handle_stroke_move(
        &mut self,
        sample: StylusSample,
        mut renderer: Option<&mut UnifiedRenderer>,
    ) -> bool {
        let canvas_pt = self.viewport.screen_to_canvas(sample.pos);

        if let Some(ref mut drag) = self.active_image_drag {
            let page_pt = PagePoint::new(
                canvas_pt.x - drag.page_origin_canvas.x,
                canvas_pt.y - drag.page_origin_canvas.y,
            );
            let Some(page) = self.doc.page(drag.page_id) else {
                return false;
            };
            let page_w = page.width_pt;
            let page_h = page.height_pt;
            let new_bounds = compute_drag_bounds(
                drag.orig_bounds,
                drag.start_pointer_page,
                page_pt,
                drag.handle,
                page_w,
                page_h,
            );
            if let Ok(prev_bounds) =
                self.doc
                    .move_resize_image(drag.page_id, drag.placement_id, new_bounds)
            {
                if let Some(r) = renderer.as_deref_mut() {
                    r.invalidate_dirty(DirtyRegion {
                        page_id: drag.page_id,
                        rect: prev_bounds.union(new_bounds),
                    });
                }
            }
            return true;
        }

        if let Some(mut session) = self.active_erase.take() {
            let page_pt = PagePoint::new(
                canvas_pt.x - session.page_origin_canvas.x,
                canvas_pt.y - session.page_origin_canvas.y,
            );
            let mode = self.tool_state.eraser_mode;
            self.run_erase_step(&mut session, page_pt, mode, renderer.as_deref_mut());
            session.prev_pt = Some(page_pt);
            self.active_erase = Some(session);
            return true;
        }

        if let Some(ref mut session) = self.active_ink {
            // Strict single-page ownership: clamp incoming move within owning page bounds.
            let local_x =
                (canvas_pt.x - session.page_origin_canvas.x).clamp(0.0, session.page_w_pt);
            let local_y =
                (canvas_pt.y - session.page_origin_canvas.y).clamp(0.0, session.page_h_pt);
            let raw = RawInputSample::new(
                local_x,
                local_y,
                f64::from(sample.pressure),
                sample.tilt_x,
                sample.tilt_y,
                sample.timestamp_ms,
            );
            session.last_sample = raw;
            self.stabilizer
                .push(raw, session.style, &mut session.points);
            self.tessellator.tessellate_incremental(
                &session.points,
                session.style,
                &mut session.mesh,
            );
            if let Some(r) = renderer {
                r.update_active_stroke(session.page_id, &session.mesh, session.style);
            }
            return true;
        }

        false
    }

    fn handle_stroke_end(
        &mut self,
        sample: StylusSample,
        mut renderer: Option<&mut UnifiedRenderer>,
    ) -> bool {
        let canvas_pt = self.viewport.screen_to_canvas(sample.pos);

        if let Some(drag) = self.active_image_drag.take() {
            let final_bounds = self
                .doc
                .page(drag.page_id)
                .and_then(|p| p.images.iter().find(|img| img.id == drag.placement_id))
                .map(|img| img.bounds);
            if let Some(new_bounds) = final_bounds {
                if new_bounds != drag.orig_bounds {
                    self.undo_stack.push(UndoCommand::TransformImage {
                        page_id: drag.page_id,
                        placement_id: drag.placement_id,
                        old_bounds: drag.orig_bounds,
                        new_bounds,
                    });
                    self.mark_dirty();
                }
            }
            return true;
        }

        if let Some(mut session) = self.active_erase.take() {
            let page_pt = PagePoint::new(
                canvas_pt.x - session.page_origin_canvas.x,
                canvas_pt.y - session.page_origin_canvas.y,
            );
            let mode = self.tool_state.eraser_mode;
            self.run_erase_step(&mut session, page_pt, mode, renderer.as_deref_mut());
            if !session.removed.is_empty() || !session.added.is_empty() {
                self.undo_stack.push(UndoCommand::EraseStrokes {
                    page_id: session.page_id,
                    removed: session.removed,
                    added: session.added,
                });
                self.mark_dirty();
            }
            self.tool_state.on_erase_gesture_complete();
            return true;
        }

        if let Some(mut session) = self.active_ink.take() {
            let local_x =
                (canvas_pt.x - session.page_origin_canvas.x).clamp(0.0, session.page_w_pt);
            let local_y =
                (canvas_pt.y - session.page_origin_canvas.y).clamp(0.0, session.page_h_pt);
            let final_raw = RawInputSample::new(
                local_x,
                local_y,
                f64::from(sample.pressure).max(0.05),
                sample.tilt_x,
                sample.tilt_y,
                sample.timestamp_ms.max(session.last_sample.timestamp_ms),
            );
            self.stabilizer
                .finish(final_raw, session.style, &mut session.points);
            self.tessellator.reset();

            let stroke_id = self.doc.allocate_stroke_id();
            let stroke = Stroke::from_points(stroke_id, session.style, session.points);
            let dirty = DirtyRegion {
                page_id: session.page_id,
                rect: stroke.bounds,
            };

            if let Some(page) = self.doc.page_mut(session.page_id) {
                page.insert_stroke_sorted(stroke.clone());
            }
            self.undo_stack.push(UndoCommand::AddStroke {
                page_id: session.page_id,
                stroke,
            });
            self.mark_dirty();

            if let Some(r) = renderer {
                r.clear_active_stroke();
                r.invalidate_dirty(dirty);
            }
            if session.is_hardware_eraser {
                self.tool_state.on_erase_gesture_complete();
            }
            return true;
        }

        false
    }

    fn flush_in_flight_interactions(&mut self, renderer: Option<&mut UnifiedRenderer>) {
        let synthetic_end = StylusSample {
            pos: self.hover_cursor.unwrap_or_default(),
            pressure: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
            timestamp_ms: 0,
            is_eraser: false,
            from_pen: false,
        };
        if let Some(ref ink) = self.active_ink {
            let end = StylusSample {
                pos: self.viewport.canvas_to_screen(CanvasPoint::new(
                    ink.page_origin_canvas.x + ink.last_sample.x,
                    ink.page_origin_canvas.y + ink.last_sample.y,
                )),
                pressure: ink.last_sample.pressure as f32,
                tilt_x: ink.last_sample.tilt_x,
                tilt_y: ink.last_sample.tilt_y,
                timestamp_ms: ink.last_sample.timestamp_ms,
                is_eraser: ink.is_hardware_eraser,
                from_pen: true,
            };
            let _ = self.handle_stroke_end(end, renderer);
        } else if self.active_erase.is_some() || self.active_image_drag.is_some() {
            let _ = self.handle_stroke_end(synthetic_end, renderer);
        }
    }

    fn run_erase_step(
        &mut self,
        session: &mut ActiveEraseSession,
        center: PagePoint,
        mode: EraserMode,
        renderer: Option<&mut UnifiedRenderer>,
    ) {
        let mut next_id = self.doc.next_stroke_id;
        let res = {
            let Some(page) = self.doc.page_mut(session.page_id) else {
                return;
            };
            erase_at(page, center, session.prev_pt, mode, &mut || {
                let id = StrokeId(next_id);
                next_id = next_id.saturating_add(1);
                id
            })
        };
        self.doc.next_stroke_id = next_id;

        if res.removed.is_empty() && res.added.is_empty() {
            return;
        }

        for s in res.removed {
            if session.added_ids.remove(&s.id) {
                session.added.retain(|a| a.id != s.id);
            } else {
                session.removed.push(s);
            }
        }
        for s in res.added {
            session.added_ids.insert(s.id);
            session.added.push(s);
        }

        session.dirty = session.dirty.union(res.dirty_rect);
        if let Some(r) = renderer {
            r.invalidate_dirty(DirtyRegion {
                page_id: session.page_id,
                rect: res.dirty_rect,
            });
        }
    }

    fn start_image_select_or_drag(
        &mut self,
        page_id: PageId,
        page_origin_canvas: CanvasPoint,
        page_pt: PagePoint,
    ) -> bool {
        let hit_radius_pt = HANDLE_HIT_RADIUS_PX / self.viewport.zoom.max(0.1);

        if let Some((sel_page, sel_id)) = self.selected_image {
            if sel_page == page_id {
                if let Some(page) = self.doc.page(page_id) {
                    if let Some(img) = page.images.iter().find(|i| i.id == sel_id) {
                        if let Some(handle) =
                            hit_test_corner_handle(img.bounds, page_pt, hit_radius_pt)
                        {
                            self.active_image_drag = Some(ActiveImageDrag {
                                page_id,
                                page_origin_canvas,
                                placement_id: img.id,
                                handle: Some(handle),
                                start_pointer_page: page_pt,
                                orig_bounds: img.bounds,
                            });
                            return true;
                        }
                    }
                }
            }
        }

        let hit = self.doc.page(page_id).and_then(|page| {
            page.images
                .iter()
                .rev()
                .find(|img| img.bounds.contains(page_pt.x, page_pt.y))
                .map(|img| (img.id, img.bounds))
        });

        match hit {
            Some((placement_id, orig_bounds)) => {
                self.selected_image = Some((page_id, placement_id));
                self.active_image_drag = Some(ActiveImageDrag {
                    page_id,
                    page_origin_canvas,
                    placement_id,
                    handle: None,
                    start_pointer_page: page_pt,
                    orig_bounds,
                });
            }
            None => {
                self.selected_image = None;
            }
        }
        true
    }

    fn delete_selected_image(&mut self, renderer: Option<&mut UnifiedRenderer>) {
        let Some((page_id, placement_id)) = self.selected_image.take() else {
            return;
        };
        let Some(page) = self.doc.page_mut(page_id) else {
            return;
        };
        let Some(index) = page.images.iter().position(|img| img.id == placement_id) else {
            return;
        };
        let placement = page.images.remove(index);
        let dirty = DirtyRegion {
            page_id,
            rect: placement.bounds,
        };
        self.undo_stack.push(UndoCommand::DeleteImage {
            page_id,
            index,
            placement,
        });
        if let Some(r) = renderer {
            r.invalidate_dirty(dirty);
        }
        self.mark_dirty();
    }

    /// Reads OS clipboard cleanly, inserting raw lossless RGBA images centered on the active page.
    pub fn paste_from_clipboard(
        &mut self,
        viewport_px: (u32, u32),
        display_scale: f32,
        renderer: Option<&mut UnifiedRenderer>,
    ) {
        match read_os_clipboard() {
            Ok(ClipboardContent::Image {
                width,
                height,
                rgba,
            }) => {
                if let Err(err) = self.paste_rgba_image(
                    width,
                    height,
                    &rgba,
                    viewport_px,
                    display_scale,
                    renderer,
                ) {
                    self.set_toast(format!("Paste image failed: {err}"), true);
                }
            }
            Ok(ClipboardContent::Text(_)) => {
                self.set_toast("Clipboard holds text, not an image", false);
            }
            Ok(ClipboardContent::Empty) => {
                self.set_toast("Clipboard is empty", false);
            }
            Err(err) => {
                self.set_toast(err, true);
            }
        }
    }

    /// Encodes `rgba` losslessly at native `(width, height)` and places it centered on the active page.
    pub fn paste_rgba_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        viewport_px: (u32, u32),
        display_scale: f32,
        renderer: Option<&mut UnifiedRenderer>,
    ) -> Result<ImagePlacement, String> {
        let asset = ImageAsset::from_rgba(width, height, rgba).map_err(|e| e.to_string())?;
        self.place_centered_image_asset(asset, viewport_px, display_scale, renderer)
    }

    fn place_centered_image_asset(
        &mut self,
        asset: ImageAsset,
        viewport_px: (u32, u32),
        display_scale: f32,
        mut renderer: Option<&mut UnifiedRenderer>,
    ) -> Result<ImagePlacement, String> {
        let asset_w_px = f64::from(asset.width_px.max(1));
        let asset_h_px = f64::from(asset.height_px.max(1));
        if let Some(r) = renderer.as_deref_mut() {
            r.ensure_image_uploaded(&asset).map_err(|e| e.to_string())?;
        }
        let asset_id = self.doc.insert_image_asset(asset);

        let page_idx = self.active_page_index(viewport_px, display_scale);
        let page = self
            .doc
            .pages
            .get(page_idx)
            .ok_or_else(|| "Document has no pages".to_owned())?;
        let page_id = page.id;
        let page_w = page.width_pt;
        let page_h = page.height_pt;

        // Convert CSS pixels (96 DPI -> 72 pt/in = 0.75 pt/px) capped to 75% of page bounds.
        let natural_w_pt = asset_w_px * 0.75;
        let natural_h_pt = asset_h_px * 0.75;
        let max_w = (page_w * MAX_PASTE_WIDTH_RATIO).max(MIN_IMAGE_EXTENT_PT);
        let max_h = (page_h * MAX_PASTE_HEIGHT_RATIO).max(MIN_IMAGE_EXTENT_PT);
        let fit_scale = (max_w / natural_w_pt).min(max_h / natural_h_pt).min(1.0);
        let disp_w = (natural_w_pt * fit_scale).max(MIN_IMAGE_EXTENT_PT);
        let disp_h = (natural_h_pt * fit_scale).max(MIN_IMAGE_EXTENT_PT);

        let min_x = (page_w - disp_w) * 0.5;
        let min_y = (page_h - disp_h) * 0.5;
        let bounds = Rect::new(min_x, min_y, min_x + disp_w, min_y + disp_h);

        let placement = self
            .doc
            .add_image_placement(page_id, asset_id, bounds)
            .map_err(|e| e.to_string())?;
        self.undo_stack.push(UndoCommand::AddImage {
            page_id,
            placement: placement.clone(),
        });
        self.selected_image = Some((page_id, placement.id));
        self.tool_state.active_tool = ActiveTool::Select;
        if let Some(r) = renderer {
            r.invalidate_dirty(DirtyRegion {
                page_id,
                rect: bounds,
            });
        }
        self.mark_dirty();
        Ok(placement)
    }

    fn insert_image_from_path(
        &mut self,
        path: &Path,
        viewport_px: (u32, u32),
        display_scale: f32,
        renderer: Option<&mut UnifiedRenderer>,
    ) {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                self.set_toast(format!("Failed reading image: {err}"), true);
                return;
            }
        };
        let is_png = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png"));
        let asset_res = if is_png {
            ImageAsset::from_png_bytes(bytes).map_err(|e| e.to_string())
        } else {
            image::load_from_memory(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|dyn_img| {
                    let rgba = dyn_img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    ImageAsset::from_rgba(w, h, rgba.as_raw()).map_err(|e| e.to_string())
                })
        };
        match asset_res {
            Ok(asset) => {
                if let Err(err) =
                    self.place_centered_image_asset(asset, viewport_px, display_scale, renderer)
                {
                    self.set_toast(format!("Failed placing image: {err}"), true);
                }
            }
            Err(err) => self.set_toast(format!("Unsupported or corrupt image: {err}"), true),
        }
    }

    fn import_pdf_from_path(&mut self, path: &Path, renderer: Option<&mut UnifiedRenderer>) {
        let raw_bytes = match fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                self.set_toast(format!("Failed reading PDF: {err}"), true);
                return;
            }
        };
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Document.pdf")
            .to_owned();
        self.try_open_pdf_bytes(file_name, raw_bytes, None, renderer);
    }

    fn submit_pdf_password(&mut self, password: String, renderer: Option<&mut UnifiedRenderer>) {
        let Some(modal) = self.pdf_password_modal.take() else {
            return;
        };
        self.try_open_pdf_bytes(modal.file_name, modal.raw_bytes, Some(password), renderer);
    }

    fn try_open_pdf_bytes(
        &mut self,
        file_name: String,
        raw_bytes: Vec<u8>,
        password: Option<String>,
        renderer: Option<&mut UnifiedRenderer>,
    ) {
        let had_attempt = password.is_some();
        match self.pdf_worker.open_document(raw_bytes.clone(), password) {
            Ok(summary) => {
                let meta = PdfAssetMeta {
                    id: summary.asset_id,
                    page_count: summary.page_count,
                    raw_bytes,
                    encrypted: summary.encrypted,
                };
                let created =
                    self.doc
                        .append_pdf_pages(meta, &summary.page_sizes_pt, summary.locked);
                if let Some(first_id) = created.first().copied() {
                    self.jump_to_page(first_id);
                }
                if let Some(r) = renderer {
                    r.clear_all_tiles();
                }
                self.pdf_worker.advance_generation();
                self.mark_dirty();
                self.set_toast(
                    format!(
                        "Imported {} PDF page(s) from {file_name}",
                        summary.page_count
                    ),
                    false,
                );
            }
            Err(GpuError::PdfPasswordRequired) => {
                self.pdf_password_modal = Some(PdfPasswordModalState {
                    file_name,
                    raw_bytes,
                    error_message: if had_attempt {
                        Some("Incorrect password, please try again.".to_owned())
                    } else {
                        None
                    },
                });
            }
            Err(GpuError::PdfLibraryMissing) => {
                self.set_toast(
                    "PDFium library not found. Run scripts/fetch-pdfium.ps1 (or .sh) to enable PDF import."
                        .to_owned(),
                    true,
                );
            }
            Err(err) => {
                self.set_toast(format!("Failed to open PDF: {err}"), true);
            }
        }
    }

    fn create_new_note(&mut self, mut renderer: Option<&mut UnifiedRenderer>) {
        self.flush_in_flight_interactions(renderer.as_deref_mut());
        if self.dirty_since.is_some() {
            self.persist_document_now();
        }
        let doc = if let Some(ref mut lib) = self.library {
            match lib.create_note("Untitled Note") {
                Ok(doc) => {
                    self.recent_notes_cache = lib.list_recent_notes();
                    doc
                }
                Err(err) => {
                    self.set_toast(format!("Failed creating note: {err}"), true);
                    return;
                }
            }
        } else {
            Document::new_untitled(
                format!("note_{}", now_unix_ms()),
                "Untitled Note",
                now_unix_ms(),
            )
        };
        self.swap_document(doc, renderer);
    }

    fn open_library_note(&mut self, note_id: &str, mut renderer: Option<&mut UnifiedRenderer>) {
        if self.doc.note_id == note_id {
            self.gallery_open = false;
            return;
        }
        self.flush_in_flight_interactions(renderer.as_deref_mut());
        if self.dirty_since.is_some() {
            self.persist_document_now();
        }
        let Some(ref lib) = self.library else {
            return;
        };
        match lib.open_note(note_id) {
            Ok(doc) => {
                self.swap_document(doc, renderer);
                self.gallery_open = false;
            }
            Err(err) => self.set_toast(format!("Failed opening note: {err}"), true),
        }
    }

    fn delete_library_note(&mut self, note_id: &str) {
        let Some(ref mut lib) = self.library else {
            return;
        };
        if let Err(err) = lib.delete_note(note_id) {
            self.set_toast(format!("Failed deleting note: {err}"), true);
            return;
        }
        self.recent_notes_cache = lib.list_recent_notes();
        if self.doc.note_id == note_id {
            if let Some(next) = self.recent_notes_cache.first().cloned() {
                if let Ok(doc) = lib.open_note(&next.note_id) {
                    self.swap_document(doc, None);
                }
            } else {
                let doc = Document::new_untitled(
                    format!("note_{}", now_unix_ms()),
                    "Untitled Note",
                    now_unix_ms(),
                );
                let _ = lib.save_note(&doc);
                self.recent_notes_cache = lib.list_recent_notes();
                self.swap_document(doc, None);
            }
        }
    }

    fn import_swnote_from_path(&mut self, path: &Path, mut renderer: Option<&mut UnifiedRenderer>) {
        self.flush_in_flight_interactions(renderer.as_deref_mut());
        if self.dirty_since.is_some() {
            self.persist_document_now();
        }
        let doc_res = if let Some(ref mut lib) = self.library {
            let res = lib.import_swnote(path);
            if res.is_ok() {
                self.recent_notes_cache = lib.list_recent_notes();
            }
            res
        } else {
            load_swnote(path)
        };
        match doc_res {
            Ok(doc) => self.swap_document(doc, renderer),
            Err(err) => self.set_toast(format!("Failed opening .swnote: {err}"), true),
        }
    }

    fn swap_document(&mut self, doc: Document, renderer: Option<&mut UnifiedRenderer>) {
        self.doc = doc;
        self.undo_stack.clear();
        self.selected_image = None;
        self.active_ink = None;
        self.active_erase = None;
        self.active_image_drag = None;
        self.dirty_since = None;
        self.last_saved_at = Some(Instant::now());
        self.viewport.zoom = 1.0;
        self.viewport.pan_x = self.last_logical_w * 0.5;
        self.viewport.pan_y = 64.0;
        if let Some(r) = renderer {
            r.clear_active_stroke();
            r.clear_all_tiles();
        }
        self.register_doc_pdfs_with_worker();
        self.pdf_worker.advance_generation();
    }

    fn jump_to_page(&mut self, page_id: PageId) {
        if let Some(idx) = self.doc.pages.iter().position(|p| p.id == page_id) {
            if let Some(rect) = self.doc.page_canvas_rect(idx) {
                self.viewport.pan_y = 64.0 - rect.min_y * self.viewport.zoom;
                self.pdf_worker.advance_generation();
            }
        }
    }

    fn zoom_centered(&mut self, factor: f64, viewport_px: (u32, u32), display_scale: f32) {
        let scale = f64::from(display_scale.max(1.0));
        let anchor = ScreenPoint::new(
            0.5 * f64::from(viewport_px.0) / scale,
            0.5 * f64::from(viewport_px.1) / scale,
        );
        self.viewport
            .set_zoom_at(anchor, self.viewport.zoom * factor);
        self.pdf_worker.advance_generation();
    }

    fn mark_dirty(&mut self) {
        self.doc.updated_unix_ms = now_unix_ms();
        if self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    fn persist_document_now(&mut self) {
        if let Some(ref mut lib) = self.library {
            match lib.save_note(&self.doc) {
                Ok(()) => {
                    self.recent_notes_cache = lib.list_recent_notes();
                    self.dirty_since = None;
                    self.last_saved_at = Some(Instant::now());
                }
                Err(err) => {
                    self.set_toast(format!("Autosave failed: {err}"), true);
                }
            }
        } else {
            self.dirty_since = None;
            self.last_saved_at = Some(Instant::now());
        }
    }

    fn persist_settings(&self) {
        let Some(ref lib) = self.library else {
            return;
        };
        let path = lib.root().join(SETTINGS_FILE_NAME);
        if let Ok(json) = serde_json::to_string_pretty(&self.settings) {
            let _ = fs::write(path, json);
        }
    }

    fn set_toast(&mut self, message: impl Into<String>, is_error: bool) {
        self.toast = Some(ToastNotice {
            message: message.into(),
            is_error,
            expires_at: Instant::now() + Duration::from_secs(4),
        });
    }
}

fn load_settings_from_library(lib: &NoteLibrary) -> Option<AppSettings> {
    let bytes = fs::read(lib.root().join(SETTINGS_FILE_NAME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Untitled Note".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn hit_test_corner_handle(bounds: Rect, pt: PagePoint, radius_pt: f64) -> Option<CornerHandle> {
    let corners = [
        (CornerHandle::TopLeft, bounds.min_x, bounds.min_y),
        (CornerHandle::TopRight, bounds.max_x, bounds.min_y),
        (CornerHandle::BottomLeft, bounds.min_x, bounds.max_y),
        (CornerHandle::BottomRight, bounds.max_x, bounds.max_y),
    ];
    let mut best = None;
    let mut best_d = radius_pt;
    for (handle, cx, cy) in corners {
        let d = ((pt.x - cx).powi(2) + (pt.y - cy).powi(2)).sqrt();
        if d <= best_d {
            best_d = d;
            best = Some(handle);
        }
    }
    best
}

fn compute_drag_bounds(
    orig: Rect,
    start_pt: PagePoint,
    cur_pt: PagePoint,
    handle: Option<CornerHandle>,
    page_w: f64,
    page_h: f64,
) -> Rect {
    let orig_w = (orig.max_x - orig.min_x).max(MIN_IMAGE_EXTENT_PT);
    let orig_h = (orig.max_y - orig.min_y).max(MIN_IMAGE_EXTENT_PT);

    let Some(corner) = handle else {
        let dx = cur_pt.x - start_pt.x;
        let dy = cur_pt.y - start_pt.y;
        let new_min_x = (orig.min_x + dx).clamp(0.0, (page_w - orig_w).max(0.0));
        let new_min_y = (orig.min_y + dy).clamp(0.0, (page_h - orig_h).max(0.0));
        return Rect::new(new_min_x, new_min_y, new_min_x + orig_w, new_min_y + orig_h);
    };

    // Anchor opposite corner and maintain strict aspect ratio during corner resize.
    let aspect = orig_w / orig_h;
    let (anchor_x, anchor_y, sign_x, sign_y) = match corner {
        CornerHandle::BottomRight => (orig.min_x, orig.min_y, 1.0_f64, 1.0_f64),
        CornerHandle::TopLeft => (orig.max_x, orig.max_y, -1.0_f64, -1.0_f64),
        CornerHandle::TopRight => (orig.min_x, orig.max_y, 1.0_f64, -1.0_f64),
        CornerHandle::BottomLeft => (orig.max_x, orig.min_y, -1.0_f64, 1.0_f64),
    };
    let raw_w = ((cur_pt.x - anchor_x) * sign_x).max(MIN_IMAGE_EXTENT_PT);
    let raw_h = ((cur_pt.y - anchor_y) * sign_y).max(MIN_IMAGE_EXTENT_PT);
    let new_w = raw_w.max(raw_h * aspect);
    let new_h = new_w / aspect;
    let other_x = anchor_x + sign_x * new_w;
    let other_y = anchor_y + sign_y * new_h;
    Rect::new(
        anchor_x.min(other_x),
        anchor_y.min(other_y),
        anchor_x.max(other_x),
        anchor_y.max(other_y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use swiftnote_core::BrushKind;

    #[test]
    fn app_controller_applies_ui_actions_sequentially_without_reentrancy() {
        let mut ctrl = AppController::new_with_library(None);
        let custom_brush = BrushStyle {
            kind: BrushKind::HighlighterRound,
            color: swiftnote_core::Color::HIGHLIGHTER_YELLOW,
            width_pt: 16.0,
            pressure_sensitive: false,
        };

        let changed = ctrl.apply_actions(
            vec![
                UiAction::SelectPenSlot(2),
                UiAction::UpdateActivePenBrush(custom_brush),
                UiAction::SetPagePolarity(PaperPolarity::Blackboard),
                UiAction::SetAutoInvertMonochromeInk(false),
                UiAction::ZoomIn,
                UiAction::AddPage,
                UiAction::RenameDocument("Linear Algebra".to_owned()),
            ],
            (1280, 800),
            1.0,
            None,
        );

        assert!(changed);
        assert_eq!(ctrl.tool_state.active_pen_slot, 2);
        assert_eq!(ctrl.tool_state.active_brush(), custom_brush);
        assert!(!ctrl.settings.auto_invert_monochrome_ink);
        assert_eq!(ctrl.doc.title, "Linear Algebra");
        assert_eq!(ctrl.doc.pages.len(), 2);
        assert!(matches!(
            ctrl.doc.pages[0].background,
            PageBackground::Procedural {
                polarity: PaperPolarity::Blackboard,
                ..
            }
        ));
        assert!(ctrl.viewport.zoom > 1.0);
    }

    #[test]
    fn ctrl_v_paste_places_image_centered_without_resampling_asset() {
        let mut ctrl = AppController::new_with_library(None);
        let width = 320;
        let height = 180;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.push((x % 256) as u8);
                rgba.push((y % 256) as u8);
                rgba.push(140);
                rgba.push(255);
            }
        }

        let res = ctrl.paste_rgba_image(width, height, &rgba, (1280, 800), 1.0, None);
        assert!(res.is_ok());
        let Ok(placement) = res else {
            return;
        };

        // Asset pixels are stored losslessly at exact original dimensions (no downscaling).
        let stored = ctrl.doc.images.get(&placement.asset_id);
        assert!(stored.is_some());
        if let Some(asset) = stored {
            assert_eq!(asset.width_px, width);
            assert_eq!(asset.height_px, height);
        }

        // Centered cleanly inside page 1.
        let page = &ctrl.doc.pages[0];
        let center_x = 0.5 * (placement.bounds.min_x + placement.bounds.max_x);
        let center_y = 0.5 * (placement.bounds.min_y + placement.bounds.max_y);
        assert!((center_x - page.width_pt * 0.5).abs() < 1e-6);
        assert!((center_y - page.height_pt * 0.5).abs() < 1e-6);
        assert_eq!(ctrl.selected_image, Some((page.id, placement.id)));
        assert_eq!(ctrl.tool_state.active_tool, ActiveTool::Select);
    }

    #[test]
    fn delete_page_clears_selected_image_and_new_note_recenters_viewport() {
        let mut ctrl = AppController::new_with_library(None);
        ctrl.ensure_initial_viewport_centered((1600, 900), 1.0);
        assert!((ctrl.viewport.pan_x - 800.0).abs() < 1e-6);

        ctrl.apply_actions(vec![UiAction::AddPage], (1600, 900), 1.0, None);
        assert_eq!(ctrl.doc.pages.len(), 2);
        let page2_id = ctrl.doc.pages[1].id;

        let rgba = vec![255u8; 4 * 4 * 4];
        let res = ctrl.paste_rgba_image(4, 4, &rgba, (1600, 900), 1.0, None);
        assert!(res.is_ok());
        let Ok(placement) = res else {
            return;
        };
        assert_eq!(ctrl.selected_image, Some((page2_id, placement.id)));

        ctrl.apply_actions(vec![UiAction::DeletePage(page2_id)], (1600, 900), 1.0, None);
        assert_eq!(ctrl.selected_image, None);

        ctrl.viewport.pan_x = 123.0;
        ctrl.viewport.zoom = 2.0;
        ctrl.apply_actions(vec![UiAction::NewNote], (1600, 900), 1.0, None);
        assert!((ctrl.viewport.zoom - 1.0).abs() < 1e-6);
        assert!((ctrl.viewport.pan_x - 800.0).abs() < 1e-6);
    }
}

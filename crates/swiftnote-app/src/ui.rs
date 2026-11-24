use egui::{
    color_picker::{color_edit_button_srgba, Alpha},
    pos2, vec2, Align, Align2, Button, Color32, CornerRadius, Frame, Id, LayerId, Layout, Margin,
    Order, Painter, Pos2, Rect, ScrollArea, Sense, Shadow, Slider, Stroke, StrokeKind, TextEdit,
    Ui, Vec2, Window,
};
use swiftnote_core::{
    ActiveTool, BrushKind, BrushStyle, CanvasPoint, Color as CoreColor, EraserMode, EraserSize,
    InputMode, PageBackground, PagePoint, PaperPolarity, PaperStandard, PaperTemplate, ScreenPoint,
    StabilizerPreset, UiTheme, DEFAULT_PALETTE,
};

use crate::controller::{AppController, ToolbarPopover, UiAction};
use crate::icons::{IconId, IconRegistry};
use crate::theme::MattePalette;

const TOP_BAR_HEIGHT: f32 = 44.0;
const GALLERY_WIDTH: f32 = 256.0;
const PAGES_SIDEBAR_WIDTH: f32 = 196.0;
const HIGHLIGHTER_ALPHA: u8 = 115;

/// Renders the entire solid-matte Swift Note UI and collects sequential [`UiAction`]s.
pub fn build_ui(
    root_ui: &mut Ui,
    controller: &AppController,
    icons: &IconRegistry,
    palette: MattePalette,
    viewport_px: (u32, u32),
    display_scale: f32,
) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let ctx = root_ui.ctx().clone();

    if controller.open_popover != ToolbarPopover::None
        && ctx.input(|i| i.key_pressed(egui::Key::Escape))
    {
        actions.push(UiAction::ClosePopovers);
    }

    draw_top_bar(root_ui, controller, icons, palette, &mut actions);

    if controller.gallery_open {
        draw_gallery_panel(root_ui, controller, icons, palette, &mut actions);
    }

    if controller.pages_sidebar_open {
        draw_pages_sidebar(root_ui, controller, icons, palette, &mut actions);
    }

    let canvas_rect = root_ui.available_rect_before_wrap();
    draw_canvas_overlay(
        &ctx,
        canvas_rect,
        controller,
        palette,
        viewport_px,
        display_scale,
    );

    if controller.open_popover != ToolbarPopover::None {
        draw_active_popover(
            &ctx,
            controller,
            palette,
            viewport_px,
            display_scale,
            &mut actions,
        );
    }

    draw_status_pill(
        &ctx,
        controller,
        icons,
        palette,
        viewport_px,
        display_scale,
        &mut actions,
    );

    if controller.toast.is_some() {
        draw_toast_banner(&ctx, controller, icons, palette, &mut actions);
    }

    if controller.pdf_password_modal.is_some() {
        draw_pdf_password_modal(&ctx, controller, palette, &mut actions);
    }

    actions
}

fn solid_card_frame(palette: MattePalette, margin: Margin) -> Frame {
    Frame::NONE
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(margin)
        .shadow(Shadow::NONE)
}

fn matte_button(
    ui: &mut Ui,
    palette: MattePalette,
    active: bool,
    button: Button<'_>,
) -> egui::Response {
    let styled = if active {
        button
            .fill(palette.accent_soft)
            .stroke(Stroke::new(1.5, palette.accent))
    } else {
        button
            .fill(palette.surface_subtle)
            .stroke(Stroke::new(1.0, palette.border))
    };
    ui.add(styled)
}

fn icon_toggle_button(
    ui: &mut Ui,
    icons: &IconRegistry,
    palette: MattePalette,
    id: IconId,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let tint = if active {
        palette.accent
    } else {
        palette.text_primary
    };
    let btn = Button::image(icons.image(id, 17.0, tint)).min_size(vec2(30.0, 28.0));
    matte_button(ui, palette, active, btn).on_hover_text(tooltip)
}

fn draw_top_bar(
    root_ui: &mut Ui,
    controller: &AppController,
    icons: &IconRegistry,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    let resp = egui::Panel::top("swiftnote_top_bar")
        .exact_size(TOP_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(
            Frame::NONE
                .fill(palette.surface)
                .inner_margin(Margin::symmetric(10, 6))
                .shadow(Shadow::NONE),
        )
        .show(root_ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                if icon_toggle_button(
                    ui,
                    icons,
                    palette,
                    IconId::Gallery,
                    controller.gallery_open,
                    "Notes Gallery (Ctrl+G)",
                )
                .clicked()
                {
                    actions.push(UiAction::ToggleGallery);
                }

                // Title buffer stored cleanly in egui temp data scoped per note ID.
                let title_buf_id = Id::unique("swiftnote_title_edit_buf");
                let title_key_id = Id::unique("swiftnote_title_edit_note_id");
                let mut title_buf = ui.ctx().data_mut(|d| {
                    let prev_note: Option<String> = d.get_temp(title_key_id);
                    if prev_note.as_deref() != Some(controller.doc.note_id.as_str()) {
                        d.insert_temp(title_key_id, controller.doc.note_id.clone());
                        d.insert_temp(title_buf_id, controller.doc.title.clone());
                    }
                    d.get_temp::<String>(title_buf_id)
                        .unwrap_or_else(|| controller.doc.title.clone())
                });

                let title_resp = ui.add(
                    TextEdit::singleline(&mut title_buf)
                        .desired_width(156.0)
                        .margin(Margin::symmetric(8, 4)),
                );
                if title_resp.changed() {
                    let clean = if title_buf.trim().is_empty() {
                        "Untitled Note".to_owned()
                    } else {
                        title_buf.clone()
                    };
                    if clean != controller.doc.title {
                        actions.push(UiAction::RenameDocument(clean));
                    }
                }
                if !title_resp.has_focus() && title_buf != controller.doc.title {
                    title_buf = controller.doc.title.clone();
                }
                ui.ctx()
                    .data_mut(|d| d.insert_temp(title_buf_id, title_buf));

                ui.add_space(4.0);

                ui.add_enabled_ui(controller.undo_stack.can_undo(), |ui| {
                    let tint = if controller.undo_stack.can_undo() {
                        palette.text_primary
                    } else {
                        palette.text_secondary
                    };
                    let btn = Button::image(icons.image(IconId::Undo, 16.0, tint))
                        .fill(palette.surface_subtle)
                        .stroke(Stroke::new(1.0, palette.border))
                        .min_size(vec2(28.0, 28.0));
                    if ui.add(btn).on_hover_text("Undo (Ctrl+Z)").clicked() {
                        actions.push(UiAction::Undo);
                    }
                });

                ui.add_enabled_ui(controller.undo_stack.can_redo(), |ui| {
                    let tint = if controller.undo_stack.can_redo() {
                        palette.text_primary
                    } else {
                        palette.text_secondary
                    };
                    let btn = Button::image(icons.image(IconId::Redo, 16.0, tint))
                        .fill(palette.surface_subtle)
                        .stroke(Stroke::new(1.0, palette.border))
                        .min_size(vec2(28.0, 28.0));
                    if ui.add(btn).on_hover_text("Redo (Ctrl+Y)").clicked() {
                        actions.push(UiAction::Redo);
                    }
                });

                ui.add_space(8.0);

                // 5 Favorite Pen Slots (swatch dot scaled by width; click active to configure)
                for (slot_idx, slot) in controller.tool_state.pen_slots.iter().enumerate() {
                    let active = controller.tool_state.active_tool == ActiveTool::Pen(slot_idx);
                    if draw_pen_slot_button(ui, palette, *slot, active, slot_idx).clicked() {
                        actions.push(UiAction::SelectPenSlot(slot_idx));
                    }
                }

                ui.add_space(2.0);

                if icon_toggle_button(
                    ui,
                    icons,
                    palette,
                    IconId::Eraser,
                    controller.tool_state.active_tool == ActiveTool::Eraser,
                    "Eraser (E) — click again for mode/size",
                )
                .clicked()
                {
                    actions.push(UiAction::SelectEraser);
                }

                if icon_toggle_button(
                    ui,
                    icons,
                    palette,
                    IconId::Select,
                    controller.tool_state.active_tool == ActiveTool::Select,
                    "Select & Resize Image (V)",
                )
                .clicked()
                {
                    actions.push(UiAction::SelectImageTool);
                }

                if controller.selected_image.is_some() {
                    let del_btn = Button::image(icons.image(IconId::Trash, 16.0, palette.danger))
                        .fill(palette.surface_subtle)
                        .stroke(Stroke::new(1.0, palette.danger))
                        .min_size(vec2(28.0, 28.0));
                    if ui
                        .add(del_btn)
                        .on_hover_text("Delete Selected Image (Del)")
                        .clicked()
                    {
                        actions.push(UiAction::DeleteSelectedImage);
                    }
                }

                // Right-aligned utilities
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    if icon_toggle_button(
                        ui,
                        icons,
                        palette,
                        IconId::Pages,
                        controller.pages_sidebar_open,
                        "Pages Sidebar (Ctrl+B)",
                    )
                    .clicked()
                    {
                        actions.push(UiAction::TogglePagesSidebar);
                    }

                    let file_open = controller.open_popover == ToolbarPopover::File;
                    let file_tint = if file_open {
                        palette.accent
                    } else {
                        palette.text_primary
                    };
                    if matte_button(
                        ui,
                        palette,
                        file_open,
                        Button::image_and_text(icons.image(IconId::File, 16.0, file_tint), "File")
                            .min_size(vec2(0.0, 28.0)),
                    )
                    .clicked()
                    {
                        actions.push(UiAction::TogglePopover(ToolbarPopover::File));
                    }

                    let paper_open = controller.open_popover == ToolbarPopover::PaperAndSettings;
                    let paper_tint = if paper_open {
                        palette.accent
                    } else {
                        palette.text_primary
                    };
                    if matte_button(
                        ui,
                        palette,
                        paper_open,
                        Button::image_and_text(
                            icons.image(IconId::Paper, 16.0, paper_tint),
                            "Paper",
                        )
                        .min_size(vec2(0.0, 28.0)),
                    )
                    .clicked()
                    {
                        actions.push(UiAction::TogglePopover(ToolbarPopover::PaperAndSettings));
                    }
                });
            });
        });

    // Crisp 1 px bottom hairline border
    let rect = resp.response.rect;
    root_ui.painter().line_segment(
        [
            pos2(rect.left(), rect.bottom()),
            pos2(rect.right(), rect.bottom()),
        ],
        Stroke::new(1.0, palette.border),
    );
}

fn draw_pen_slot_button(
    ui: &mut Ui,
    palette: MattePalette,
    style: BrushStyle,
    active: bool,
    slot_idx: usize,
) -> egui::Response {
    let desired = vec2(30.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let bg = if active {
        palette.accent_soft
    } else if response.hovered() {
        palette.surface_hover
    } else {
        palette.surface_subtle
    };
    let border_stroke = if active {
        Stroke::new(1.5, palette.accent)
    } else {
        Stroke::new(1.0, palette.border)
    };

    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(5),
        bg,
        border_stroke,
        StrokeKind::Inside,
    );

    let dot_radius = (2.5 + (style.width_pt * 0.35)).clamp(2.5, 8.0);
    let swatch = Color32::from_rgb(style.color.r, style.color.g, style.color.b);
    let center = rect.center();

    if style.is_highlighter() {
        let mark_rect = Rect::from_center_size(center, vec2(14.0, 7.0));
        painter.rect_filled(mark_rect, CornerRadius::same(2), swatch);
        painter.rect_stroke(
            mark_rect,
            CornerRadius::same(2),
            Stroke::new(1.0, palette.border),
            StrokeKind::Inside,
        );
    } else {
        painter.circle_filled(center, dot_radius, swatch);
        painter.circle_stroke(center, dot_radius, Stroke::new(1.0, palette.border));
        if matches!(style.kind, BrushKind::Pencil) {
            painter.circle_stroke(
                center,
                (dot_radius + 2.0).min(10.5),
                Stroke::new(1.0, palette.text_secondary),
            );
        }
    }

    let label = match style.kind {
        BrushKind::Pen | BrushKind::Calligraphy { .. } => "Pen",
        BrushKind::Pencil => "Pencil",
        BrushKind::HighlighterRound | BrushKind::HighlighterSquare => "Highlighter",
    };
    response.on_hover_text(format!(
        "{label} Slot {} ({:.1} pt) — click again to customize",
        slot_idx + 1,
        style.width_pt
    ))
}

fn draw_gallery_panel(
    root_ui: &mut Ui,
    controller: &AppController,
    icons: &IconRegistry,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    let resp = egui::Panel::left("swiftnote_gallery_panel")
        .exact_size(GALLERY_WIDTH)
        .show_separator_line(false)
        .frame(
            Frame::NONE
                .fill(palette.surface)
                .inner_margin(Margin::same(12))
                .shadow(Shadow::NONE),
        )
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Notes");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let close_btn =
                        Button::image(icons.image(IconId::Close, 15.0, palette.text_secondary))
                            .fill(palette.surface_subtle)
                            .stroke(Stroke::new(1.0, palette.border));
                    if ui.add(close_btn).clicked() {
                        actions.push(UiAction::ToggleGallery);
                    }
                    let new_btn = Button::image_and_text(
                        icons.image(IconId::Plus, 15.0, palette.surface),
                        "New Note",
                    )
                    .fill(palette.accent)
                    .stroke(Stroke::NONE);
                    if ui.add(new_btn).clicked() {
                        actions.push(UiAction::NewNote);
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            ScrollArea::vertical()
                .id_salt("gallery_notes_scroll")
                .show(ui, |ui| {
                    let recents = controller.recent_notes();
                    if recents.is_empty() {
                        ui.label("No saved notes yet.");
                        return;
                    }
                    for entry in recents {
                        let is_current = entry.note_id == controller.doc.note_id;
                        let card_fill = if is_current {
                            palette.accent_soft
                        } else {
                            palette.surface_subtle
                        };
                        let card_stroke = if is_current {
                            Stroke::new(1.5, palette.accent)
                        } else {
                            Stroke::new(1.0, palette.border)
                        };
                        Frame::NONE
                            .fill(card_fill)
                            .stroke(card_stroke)
                            .corner_radius(CornerRadius::same(5))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        let open_btn = Button::new(&entry.title)
                                            .fill(Color32::TRANSPARENT)
                                            .stroke(Stroke::NONE);
                                        if ui.add(open_btn).clicked() {
                                            actions.push(UiAction::OpenLibraryNote(
                                                entry.note_id.clone(),
                                            ));
                                        }
                                        ui.small(format!("{} page(s)", entry.page_count));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let del = Button::image(icons.image(
                                            IconId::Trash,
                                            14.0,
                                            palette.text_secondary,
                                        ))
                                        .fill(palette.surface)
                                        .stroke(Stroke::new(1.0, palette.border));
                                        if ui.add(del).on_hover_text("Delete note").clicked() {
                                            actions.push(UiAction::DeleteLibraryNote(
                                                entry.note_id.clone(),
                                            ));
                                        }
                                    });
                                });
                            });
                        ui.add_space(4.0);
                    }
                });
        });

    let rect = resp.response.rect;
    root_ui.painter().line_segment(
        [
            pos2(rect.right(), rect.top()),
            pos2(rect.right(), rect.bottom()),
        ],
        Stroke::new(1.0, palette.border),
    );
}

fn draw_pages_sidebar(
    root_ui: &mut Ui,
    controller: &AppController,
    icons: &IconRegistry,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    let resp = egui::Panel::right("swiftnote_pages_sidebar")
        .exact_size(PAGES_SIDEBAR_WIDTH)
        .show_separator_line(false)
        .frame(
            Frame::NONE
                .fill(palette.surface)
                .inner_margin(Margin::same(12))
                .shadow(Shadow::NONE),
        )
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("Pages ({})", controller.doc.pages.len()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let add_btn = Button::image_and_text(
                        icons.image(IconId::Plus, 14.0, palette.surface),
                        "Add",
                    )
                    .fill(palette.accent)
                    .stroke(Stroke::NONE);
                    if ui.add(add_btn).clicked() {
                        actions.push(UiAction::AddPage);
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            let can_delete = controller.doc.pages.len() > 1;
            ScrollArea::vertical()
                .id_salt("pages_list_scroll")
                .show(ui, |ui| {
                    for (idx, page) in controller.doc.pages.iter().enumerate() {
                        let tag = match page.background {
                            PageBackground::Procedural { polarity, .. } => match polarity {
                                PaperPolarity::Light => "Light Paper".to_owned(),
                                PaperPolarity::Blackboard => "Blackboard".to_owned(),
                            },
                            PageBackground::Pdf { page_index, .. } => {
                                format!("PDF p.{}", page_index + 1)
                            }
                            PageBackground::CustomImage { .. } => "Image Page".to_owned(),
                        };

                        Frame::NONE
                            .fill(palette.surface_subtle)
                            .stroke(Stroke::new(1.0, palette.border))
                            .corner_radius(CornerRadius::same(5))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        if ui
                                            .add(
                                                Button::new(format!("Page {}", idx + 1))
                                                    .fill(Color32::TRANSPARENT)
                                                    .stroke(Stroke::NONE),
                                            )
                                            .clicked()
                                        {
                                            actions.push(UiAction::JumpToPage(page.id));
                                        }
                                        ui.small(format!("{tag} · {} strokes", page.strokes.len()));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.add_enabled_ui(can_delete, |ui| {
                                            let del = Button::image(icons.image(
                                                IconId::Trash,
                                                14.0,
                                                palette.text_secondary,
                                            ))
                                            .fill(palette.surface)
                                            .stroke(Stroke::new(1.0, palette.border));
                                            if ui.add(del).on_hover_text("Delete page").clicked() {
                                                actions.push(UiAction::DeletePage(page.id));
                                            }
                                        });
                                    });
                                });
                            });
                        ui.add_space(4.0);
                    }
                });
        });

    let rect = resp.response.rect;
    root_ui.painter().line_segment(
        [
            pos2(rect.left(), rect.top()),
            pos2(rect.left(), rect.bottom()),
        ],
        Stroke::new(1.0, palette.border),
    );
}

fn draw_active_popover(
    ctx: &egui::Context,
    controller: &AppController,
    palette: MattePalette,
    viewport_px: (u32, u32),
    display_scale: f32,
    actions: &mut Vec<UiAction>,
) {
    let scale = f64::from(display_scale.max(1.0));
    let logical_w = (f64::from(viewport_px.0) / scale) as f32;

    let (anchor, offset, width) = match controller.open_popover {
        ToolbarPopover::None => return,
        ToolbarPopover::Pen => (
            Align2::LEFT_TOP,
            vec2((logical_w * 0.24).max(200.0), 50.0),
            296.0,
        ),
        ToolbarPopover::Eraser => (
            Align2::LEFT_TOP,
            vec2((logical_w * 0.34).max(260.0), 50.0),
            240.0,
        ),
        ToolbarPopover::PaperAndSettings => (Align2::RIGHT_TOP, vec2(-80.0, 50.0), 320.0),
        ToolbarPopover::File => (Align2::RIGHT_TOP, vec2(-12.0, 50.0), 224.0),
    };

    egui::Area::new(Id::unique("swiftnote_toolbar_popover"))
        .order(Order::Foreground)
        .anchor(anchor, offset)
        .show(ctx, |ui| {
            solid_card_frame(palette, Margin::same(12)).show(ui, |ui| {
                ui.set_width(width);
                match controller.open_popover {
                    ToolbarPopover::None => {}
                    ToolbarPopover::Pen => draw_pen_popover(ui, controller, palette, actions),
                    ToolbarPopover::Eraser => draw_eraser_popover(ui, controller, palette, actions),
                    ToolbarPopover::PaperAndSettings => draw_paper_settings_popover(
                        ui,
                        controller,
                        palette,
                        viewport_px,
                        display_scale,
                        actions,
                    ),
                    ToolbarPopover::File => draw_file_popover(ui, palette, actions),
                }
            });
        });
}

fn draw_pen_popover(
    ui: &mut Ui,
    controller: &AppController,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    let orig = controller.tool_state.active_brush();
    let mut brush = orig;

    ui.horizontal(|ui| {
        ui.strong(format!(
            "Pen Slot {}",
            controller.tool_state.active_pen_slot + 1
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Close").clicked() {
                actions.push(UiAction::ClosePopovers);
            }
        });
    });
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        let is_pen = matches!(brush.kind, BrushKind::Pen | BrushKind::Calligraphy { .. });
        let is_pencil = matches!(brush.kind, BrushKind::Pencil);
        let is_hl = brush.is_highlighter();

        if matte_button(ui, palette, is_pen, Button::new("Pen")).clicked() {
            brush.kind = BrushKind::Pen;
            brush.color.a = 255;
            brush.pressure_sensitive = true;
        }
        if matte_button(ui, palette, is_pencil, Button::new("Pencil")).clicked() {
            brush.kind = BrushKind::Pencil;
            brush.color.a = 255;
            brush.pressure_sensitive = true;
        }
        if matte_button(ui, palette, is_hl, Button::new("Highlighter")).clicked() {
            brush.kind = BrushKind::HighlighterRound;
            brush.color.a = HIGHLIGHTER_ALPHA;
            brush.pressure_sensitive = false;
            if brush.width_pt < 8.0 {
                brush.width_pt = 16.0;
            }
        }
    });

    ui.add_space(8.0);
    ui.label("Stroke Width (pt)");
    ui.add(Slider::new(&mut brush.width_pt, 0.5..=30.0).fixed_decimals(1));
    ui.horizontal(|ui| {
        for preset in [1.0_f32, 2.2, 4.0, 8.0, 16.0] {
            let selected = (brush.width_pt - preset).abs() < 0.15;
            if matte_button(ui, palette, selected, Button::new(format!("{preset:.1}"))).clicked() {
                brush.width_pt = preset;
            }
        }
    });

    ui.add_space(6.0);
    ui.checkbox(&mut brush.pressure_sensitive, "Pressure sensitivity");

    ui.add_space(8.0);
    ui.label("Color Palette");
    egui::Grid::new("pen_color_palette_grid")
        .num_columns(8)
        .spacing(vec2(5.0, 5.0))
        .show(ui, |ui| {
            for (idx, (name, col)) in DEFAULT_PALETTE.iter().enumerate() {
                let swatch = Color32::from_rgb(col.r, col.g, col.b);
                let selected =
                    brush.color.r == col.r && brush.color.g == col.g && brush.color.b == col.b;
                let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                let stroke = if selected {
                    Stroke::new(2.0, palette.accent)
                } else {
                    Stroke::new(1.0, palette.border)
                };
                ui.painter().rect(
                    rect,
                    CornerRadius::same(4),
                    swatch,
                    stroke,
                    StrokeKind::Inside,
                );
                if resp.on_hover_text(*name).clicked() {
                    let target_alpha = if brush.is_highlighter() {
                        HIGHLIGHTER_ALPHA
                    } else {
                        255
                    };
                    brush.color = CoreColor::rgba(col.r, col.g, col.b, target_alpha);
                }
                if idx % 8 == 7 {
                    ui.end_row();
                }
            }
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Custom color:");
        let mut custom = Color32::from_rgb(brush.color.r, brush.color.g, brush.color.b);
        if color_edit_button_srgba(ui, &mut custom, Alpha::Opaque).changed() {
            let target_alpha = if brush.is_highlighter() {
                HIGHLIGHTER_ALPHA
            } else {
                255
            };
            brush.color = CoreColor::rgba(custom.r(), custom.g(), custom.b(), target_alpha);
        }
    });

    if brush != orig {
        actions.push(UiAction::UpdateActivePenBrush(brush));
    }
}

fn draw_eraser_popover(
    ui: &mut Ui,
    controller: &AppController,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    ui.horizontal(|ui| {
        ui.strong("Eraser Mode");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Close").clicked() {
                actions.push(UiAction::ClosePopovers);
            }
        });
    });
    ui.add_space(6.0);

    let mode = controller.tool_state.eraser_mode;
    let modes = [
        ("Whole Stroke", EraserMode::Stroke),
        ("Segment — Small", EraserMode::Segment(EraserSize::Small)),
        ("Segment — Medium", EraserMode::Segment(EraserSize::Medium)),
        ("Segment — Large", EraserMode::Segment(EraserSize::Large)),
    ];
    for (label, target) in modes {
        if matte_button(
            ui,
            palette,
            mode == target,
            Button::new(label).min_size(vec2(ui.available_width(), 26.0)),
        )
        .clicked()
        {
            actions.push(UiAction::SetEraserMode(target));
        }
    }

    ui.add_space(8.0);
    let mut back_to_pen = controller.settings.back_to_pen_after_erase;
    if ui
        .checkbox(&mut back_to_pen, "Back to previous pen after erasing")
        .changed()
    {
        actions.push(UiAction::SetBackToPen(back_to_pen));
    }
}

fn draw_paper_settings_popover(
    ui: &mut Ui,
    controller: &AppController,
    palette: MattePalette,
    viewport_px: (u32, u32),
    display_scale: f32,
    actions: &mut Vec<UiAction>,
) {
    ui.horizontal(|ui| {
        ui.strong("Paper & Preferences");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Close").clicked() {
                actions.push(UiAction::ClosePopovers);
            }
        });
    });
    ui.add_space(6.0);

    let active_idx = controller.active_page_index(viewport_px, display_scale);
    let active_bg = controller.doc.pages.get(active_idx).map(|p| &p.background);
    let (is_procedural, cur_template, cur_polarity) = match active_bg {
        Some(PageBackground::Procedural { template, polarity }) => {
            (true, Some(*template), Some(*polarity))
        }
        _ => (false, None, None),
    };

    ui.label("Paper Background");
    if !is_procedural {
        ui.small("Current page is a PDF or Image page.");
    }
    ui.add_enabled_ui(is_procedural, |ui| {
        ui.horizontal(|ui| {
            if matte_button(
                ui,
                palette,
                cur_polarity == Some(PaperPolarity::Light),
                Button::new("Light Paper"),
            )
            .clicked()
            {
                actions.push(UiAction::SetPagePolarity(PaperPolarity::Light));
            }
            if matte_button(
                ui,
                palette,
                cur_polarity == Some(PaperPolarity::Blackboard),
                Button::new("Blackboard"),
            )
            .clicked()
            {
                actions.push(UiAction::SetPagePolarity(PaperPolarity::Blackboard));
            }
        });

        ui.add_space(4.0);
        ui.label("Paper Ruling");
        let templates = [
            ("Blank", PaperTemplate::Blank),
            ("Graph", PaperTemplate::Graph),
            ("Isometric", PaperTemplate::Isometric),
            ("Ruled 10", PaperTemplate::ruled_10()),
            ("Ruled 15", PaperTemplate::ruled_15()),
            ("Ruled 20", PaperTemplate::ruled_20()),
            ("Ruled 30", PaperTemplate::ruled_30()),
            ("Ruled 40", PaperTemplate::ruled_40()),
            ("Math 10", PaperTemplate::math_10()),
            ("Math 15", PaperTemplate::math_15()),
            ("Math 20", PaperTemplate::math_20()),
            ("Math 30", PaperTemplate::math_30()),
            ("Math 40", PaperTemplate::math_40()),
            ("Dotted 10", PaperTemplate::dotted_10()),
            ("Dotted 20", PaperTemplate::dotted_20()),
        ];
        egui::Grid::new("paper_template_grid")
            .num_columns(3)
            .spacing(vec2(4.0, 4.0))
            .show(ui, |ui| {
                for (i, (label, tpl)) in templates.iter().enumerate() {
                    if matte_button(
                        ui,
                        palette,
                        cur_template == Some(*tpl),
                        Button::new(*label).min_size(vec2(88.0, 24.0)),
                    )
                    .clicked()
                    {
                        actions.push(UiAction::SetPageTemplate(*tpl));
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });

        ui.add_space(4.0);
        ui.label("Page Size");
        ui.horizontal(|ui| {
            for (label, std) in [
                ("A4", PaperStandard::A4),
                ("Letter", PaperStandard::Letter),
                ("Legal", PaperStandard::Legal),
                ("Square", PaperStandard::Square),
            ] {
                if matte_button(
                    ui,
                    palette,
                    controller.doc.default_standard == std,
                    Button::new(label),
                )
                .clicked()
                {
                    actions.push(UiAction::SetPageStandard(std));
                }
            }
        });
    });

    ui.add_space(6.0);
    let mut auto_invert = controller.settings.auto_invert_monochrome_ink;
    if ui
        .checkbox(&mut auto_invert, "Auto-invert B/W ink on Blackboard")
        .changed()
    {
        actions.push(UiAction::SetAutoInvertMonochromeInk(auto_invert));
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label("App Theme");
    ui.horizontal(|ui| {
        for (label, theme) in [
            ("Light", UiTheme::Light),
            ("Dark", UiTheme::Dark),
            ("System", UiTheme::System),
        ] {
            if matte_button(
                ui,
                palette,
                controller.settings.ui_theme == theme,
                Button::new(label),
            )
            .clicked()
            {
                actions.push(UiAction::SetUiTheme(theme));
            }
        }
    });

    ui.add_space(4.0);
    ui.label("Input Mode");
    ui.horizontal_wrapped(|ui| {
        for (label, mode) in [
            ("Digitizer Pen", InputMode::DigitizerPen),
            ("Pen (No Pressure)", InputMode::DigitizerPenNoPressure),
            ("Capacitive / Finger", InputMode::FingerCapacitivePen),
        ] {
            if matte_button(
                ui,
                palette,
                controller.settings.input_mode == mode,
                Button::new(label),
            )
            .clicked()
            {
                actions.push(UiAction::SetInputMode(mode));
            }
        }
    });

    ui.add_space(4.0);
    ui.label("Stroke Stabilizer");
    ui.horizontal(|ui| {
        for (label, preset) in [
            ("Off", StabilizerPreset::Off),
            ("Light", StabilizerPreset::Light),
            ("Medium", StabilizerPreset::Medium),
            ("Strong", StabilizerPreset::Strong),
        ] {
            if matte_button(
                ui,
                palette,
                controller.settings.stabilizer_preset == preset,
                Button::new(label),
            )
            .clicked()
            {
                actions.push(UiAction::SetStabilizerPreset(preset));
            }
        }
    });
}

fn draw_file_popover(ui: &mut Ui, palette: MattePalette, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.strong("Document & Assets");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Close").clicked() {
                actions.push(UiAction::ClosePopovers);
            }
        });
    });
    ui.add_space(6.0);

    let full_w = ui.available_width();
    let entries = [
        ("New Note (Ctrl+N)", UiAction::NewNote),
        ("Open .swnote... (Ctrl+O)", UiAction::ImportSwnote),
        (
            "Save Copy (.swnote)... (Ctrl+S)",
            UiAction::ExportSwnoteCopy,
        ),
        ("Import PDF...", UiAction::ImportPdf),
        ("Insert Image...", UiAction::InsertImageFromDisk),
        ("Paste Image (Ctrl+V)", UiAction::PasteFromClipboard),
    ];
    for (label, action) in entries {
        if matte_button(
            ui,
            palette,
            false,
            Button::new(label).min_size(vec2(full_w, 26.0)),
        )
        .clicked()
        {
            actions.push(action);
        }
    }
}

fn draw_status_pill(
    ctx: &egui::Context,
    controller: &AppController,
    icons: &IconRegistry,
    palette: MattePalette,
    viewport_px: (u32, u32),
    display_scale: f32,
    actions: &mut Vec<UiAction>,
) {
    let page_total = controller.doc.pages.len().max(1);
    let page_cur = controller.active_page_index(viewport_px, display_scale) + 1;
    let zoom_pct = (controller.viewport.zoom * 100.0).round() as i32;
    let saved_label = if controller.is_dirty() {
        "Saving..."
    } else if controller.last_saved_elapsed_secs().is_some() {
        "Saved"
    } else {
        "Ready"
    };
    let saved_color = if controller.is_dirty() {
        palette.text_secondary
    } else {
        palette.success
    };

    egui::Area::new(Id::unique("swiftnote_status_pill"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -14.0))
        .show(ctx, |ui| {
            solid_card_frame(palette, Margin::symmetric(10, 5)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(format!("Page {page_cur}/{page_total}"));
                    ui.separator();

                    let zoom_out =
                        Button::image(icons.image(IconId::ZoomOut, 14.0, palette.text_primary))
                            .fill(palette.surface_subtle)
                            .stroke(Stroke::new(1.0, palette.border));
                    if ui
                        .add(zoom_out)
                        .on_hover_text("Zoom Out (Ctrl+-)")
                        .clicked()
                    {
                        actions.push(UiAction::ZoomOut);
                    }

                    if ui
                        .add(
                            Button::new(format!("{zoom_pct}%"))
                                .fill(palette.surface_subtle)
                                .stroke(Stroke::new(1.0, palette.border)),
                        )
                        .on_hover_text("Reset Zoom to 100% (Ctrl+0)")
                        .clicked()
                    {
                        actions.push(UiAction::ZoomReset);
                    }

                    let zoom_in =
                        Button::image(icons.image(IconId::ZoomIn, 14.0, palette.text_primary))
                            .fill(palette.surface_subtle)
                            .stroke(Stroke::new(1.0, palette.border));
                    if ui.add(zoom_in).on_hover_text("Zoom In (Ctrl++)").clicked() {
                        actions.push(UiAction::ZoomIn);
                    }

                    ui.separator();
                    ui.colored_label(saved_color, saved_label);
                });
            });
        });
}

fn draw_toast_banner(
    ctx: &egui::Context,
    controller: &AppController,
    icons: &IconRegistry,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    let Some(toast) = &controller.toast else {
        return;
    };
    let border_color = if toast.is_error {
        palette.danger
    } else {
        palette.accent
    };
    egui::Area::new(Id::unique("swiftnote_toast_banner"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -18.0))
        .show(ctx, |ui| {
            Frame::NONE
                .fill(palette.surface)
                .stroke(Stroke::new(1.0, border_color))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(12, 8))
                .shadow(Shadow::NONE)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let text_color = if toast.is_error {
                            palette.danger
                        } else {
                            palette.text_primary
                        };
                        ui.colored_label(text_color, &toast.message);
                        let dismiss =
                            Button::image(icons.image(IconId::Close, 13.0, palette.text_secondary))
                                .fill(palette.surface_subtle)
                                .stroke(Stroke::new(1.0, palette.border));
                        if ui.add(dismiss).clicked() {
                            actions.push(UiAction::DismissToast);
                        }
                    });
                });
        });
}

fn draw_pdf_password_modal(
    ctx: &egui::Context,
    controller: &AppController,
    palette: MattePalette,
    actions: &mut Vec<UiAction>,
) {
    let Some(modal) = &controller.pdf_password_modal else {
        return;
    };

    Window::new("Unlock Encrypted PDF")
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .frame(solid_card_frame(palette, Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(format!("Enter password to open \"{}\":", modal.file_name));
            if let Some(ref err) = modal.error_message {
                ui.add_space(4.0);
                ui.colored_label(palette.danger, err);
            }
            ui.add_space(8.0);

            let pw_id = Id::unique("swiftnote_pdf_password_input");
            let mut pw_buf = ui
                .ctx()
                .data_mut(|d| d.get_temp::<String>(pw_id).unwrap_or_default());

            let edit_resp = ui.add(
                TextEdit::singleline(&mut pw_buf)
                    .password(true)
                    .desired_width(ui.available_width()),
            );
            let submit_enter =
                edit_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            ui.ctx().data_mut(|d| d.insert_temp(pw_id, pw_buf.clone()));

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        Button::new("Unlock")
                            .fill(palette.accent)
                            .stroke(Stroke::NONE),
                    )
                    .clicked()
                    || submit_enter
                {
                    ui.ctx().data_mut(|d| d.insert_temp(pw_id, String::new()));
                    actions.push(UiAction::SubmitPdfPassword(pw_buf));
                }
                if matte_button(ui, palette, false, Button::new("Cancel")).clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(pw_id, String::new()));
                    actions.push(UiAction::CancelPdfPassword);
                }
            });
        });
}

fn draw_canvas_overlay(
    ctx: &egui::Context,
    canvas_rect: Rect,
    controller: &AppController,
    palette: MattePalette,
    viewport_px: (u32, u32),
    display_scale: f32,
) {
    if canvas_rect.width() <= 0.0 || canvas_rect.height() <= 0.0 {
        return;
    }

    let painter: Painter = ctx
        .layer_painter(LayerId::new(
            Order::Background,
            Id::unique("swiftnote_canvas_overlay"),
        ))
        .with_clip_rect(canvas_rect);

    let scale = f64::from(display_scale.max(1.0));
    let logical_w = f64::from(viewport_px.0) / scale;
    let logical_h = f64::from(viewport_px.1) / scale;
    let tl = controller
        .viewport
        .screen_to_canvas(ScreenPoint::new(0.0, 0.0));
    let br = controller
        .viewport
        .screen_to_canvas(ScreenPoint::new(logical_w, logical_h));
    let vis_canvas = swiftnote_core::Rect::new(
        tl.x.min(br.x),
        tl.y.min(br.y),
        tl.x.max(br.x),
        tl.y.max(br.y),
    );

    for (page_id, page_canvas_rect) in controller.doc.visible_pages(vis_canvas) {
        let s_tl = controller.viewport.canvas_to_screen(CanvasPoint::new(
            page_canvas_rect.min_x,
            page_canvas_rect.min_y,
        ));
        let s_br = controller.viewport.canvas_to_screen(CanvasPoint::new(
            page_canvas_rect.max_x,
            page_canvas_rect.max_y,
        ));
        let screen_rect = Rect::from_min_max(
            pos2(s_tl.x as f32, s_tl.y as f32),
            pos2(s_br.x as f32, s_br.y as f32),
        );
        painter.rect_stroke(
            screen_rect,
            CornerRadius::ZERO,
            Stroke::new(1.0, palette.border),
            StrokeKind::Outside,
        );

        if let Some(page) = controller.doc.page(page_id) {
            if let PageBackground::Pdf {
                locked_placeholder: true,
                ..
            } = page.background
            {
                let chip = Rect::from_center_size(screen_rect.center(), vec2(260.0, 34.0));
                painter.rect(
                    chip,
                    CornerRadius::same(6),
                    palette.surface,
                    Stroke::new(1.0, palette.border),
                    StrokeKind::Inside,
                );
                painter.text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    "PDF page locked / PDFium missing",
                    egui::FontId::proportional(13.0),
                    palette.text_secondary,
                );
            }
        }
    }

    // Selected image border + 8x8 corner resize squares
    if let Some((sel_page, sel_id)) = controller.selected_image {
        if let Some(idx) = controller.doc.pages.iter().position(|p| p.id == sel_page) {
            if let (Some(page_rect), Some(img)) = (
                controller.doc.page_canvas_rect(idx),
                controller.doc.pages[idx]
                    .images
                    .iter()
                    .find(|i| i.id == sel_id),
            ) {
                let to_pos = |pt: PagePoint| -> Pos2 {
                    let s = controller.viewport.canvas_to_screen(CanvasPoint::new(
                        page_rect.min_x + pt.x,
                        page_rect.min_y + pt.y,
                    ));
                    pos2(s.x as f32, s.y as f32)
                };
                let min_p = to_pos(PagePoint::new(img.bounds.min_x, img.bounds.min_y));
                let max_p = to_pos(PagePoint::new(img.bounds.max_x, img.bounds.max_y));
                let img_screen = Rect::from_min_max(min_p, max_p);
                painter.rect_stroke(
                    img_screen,
                    CornerRadius::ZERO,
                    Stroke::new(1.5, palette.accent),
                    StrokeKind::Inside,
                );
                for corner in [
                    img_screen.left_top(),
                    img_screen.right_top(),
                    img_screen.left_bottom(),
                    img_screen.right_bottom(),
                ] {
                    let h_rect = Rect::from_center_size(corner, vec2(8.0, 8.0));
                    painter.rect(
                        h_rect,
                        CornerRadius::same(1),
                        palette.surface,
                        Stroke::new(1.5, palette.accent),
                        StrokeKind::Inside,
                    );
                }
            }
        }
    }

    // Crisp nib / eraser hover cursor ring when pointer is over the canvas
    if controller.open_popover == ToolbarPopover::None {
        if let Some(hover) = controller.hover_cursor {
            let p = pos2(hover.x as f32, hover.y as f32);
            if canvas_rect.contains(p) {
                match controller.tool_state.active_tool {
                    ActiveTool::Eraser => {
                        let r = ((controller.tool_state.eraser_mode.radius_pt()
                            * controller.viewport.zoom) as f32)
                            .clamp(3.0, 120.0);
                        painter.circle_stroke(p, r + 1.0, Stroke::new(1.0, palette.surface));
                        painter.circle_stroke(p, r, Stroke::new(1.0, palette.text_primary));
                    }
                    ActiveTool::Pen(_) => {
                        let r = ((f64::from(controller.tool_state.active_brush().width_pt)
                            * 0.5
                            * controller.viewport.zoom) as f32)
                            .clamp(2.0, 64.0);
                        painter.circle_stroke(p, r, Stroke::new(1.0, palette.text_secondary));
                    }
                    ActiveTool::Select | ActiveTool::Laser => {}
                }
            }
        }
    }
}

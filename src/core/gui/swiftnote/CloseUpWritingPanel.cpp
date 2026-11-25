#include "CloseUpWritingPanel.h"

#include <algorithm>
#include <array>
#include <cstdio>

#include "control/Control.h"
#include "control/ToolHandler.h"
#include "gui/MainWindow.h"
#include "gui/PageView.h"
#include "gui/XournalView.h"
#include "gui/inputdevices/DeviceId.h"
#include "gui/inputdevices/PositionInputData.h"
#include "gui/swiftnote/SwiftBridge.h"
#include "gui/swiftnote/SwiftShell.h"
#include "model/Point.h"
#include "model/XojPage.h"
#include "view/DocumentView.h"

namespace swiftnote {

namespace {

void setCssClass(GtkWidget* w, const char* cls, bool enabled) {
    if (!w) {
        return;
    }
    GtkStyleContext* sc = gtk_widget_get_style_context(w);
    if (enabled) {
        gtk_style_context_add_class(sc, cls);
    } else {
        gtk_style_context_remove_class(sc, cls);
    }
}

double readEventPressure(GdkEvent* ev) {
    gdouble p = Point::NO_PRESSURE;
    if (gdk_event_get_axis(ev, GDK_AXIS_PRESSURE, &p)) {
        return p;
    }
    return Point::NO_PRESSURE;
}

}  // namespace

CloseUpWritingPanel::CloseUpWritingPanel(SwiftShell* shell, Control* control): shell(shell), control(control) {
    const auto& cfg = this->shell->getSettings();
    this->zoomFactor = std::clamp(cfg.closeUpZoomFactor, 1.5, 5.0);
    this->autoAdvance = cfg.closeUpAutoAdvance;
    this->marginTriggerRatio = std::clamp(cfg.closeUpMarginTriggerRatio, 0.55, 0.92);
    buildUi();
}

CloseUpWritingPanel::~CloseUpWritingPanel() { cancelAutoAdvance(); }

void CloseUpWritingPanel::buildUi() {
    this->rootWidget = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_widget_set_name(this->rootWidget, "swiftCloseUpPanel");

    // Header bar
    GtkWidget* header = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_name(header, "swiftCloseUpHeader");

    GtkWidget* title = gtk_label_new("Vùng viết phóng to (Close-up)");
    setCssClass(title, "swift-doc-title", true);
    gtk_box_pack_start(GTK_BOX(header), title, FALSE, FALSE, 4);

    auto addNavBtn = [&](const char* label, const char* tip, GCallback cb) {
        GtkWidget* b = gtk_button_new_with_label(label);
        gtk_button_set_relief(GTK_BUTTON(b), GTK_RELIEF_NONE);
        gtk_widget_set_can_focus(b, FALSE);
        gtk_widget_set_tooltip_text(b, tip);
        g_signal_connect(b, "clicked", cb, this);
        gtk_box_pack_start(GTK_BOX(header), b, FALSE, FALSE, 0);
    };

    addNavBtn("◀ Trái", "Dịch khung sang trái", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                  static_cast<CloseUpWritingPanel*>(selfPtr)->moveLeft();
              }));
    addNavBtn("▶ Phải", "Dịch khung sang phải", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                  static_cast<CloseUpWritingPanel*>(selfPtr)->moveRight();
              }));
    addNavBtn("↵ Xuống dòng", "Đưa khung về đầu dòng kế tiếp", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                  static_cast<CloseUpWritingPanel*>(selfPtr)->nextLine();
              }));

    // Zoom segmented presets
    GtkWidget* zoomSeg = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 2);
    setCssClass(zoomSeg, "swift-segmented", true);
    const std::array<std::pair<const char*, double>, 4> zooms = {{
            {"2.0x", 2.0},
            {"2.5x", 2.5},
            {"3.0x", 3.0},
            {"4.0x", 4.0},
    }};
    for (const auto& [lbl, factor]: zooms) {
        GtkWidget* zb = gtk_button_new_with_label(lbl);
        gtk_widget_set_can_focus(zb, FALSE);
        g_object_set_data(G_OBJECT(zb), "zoom-x10", GINT_TO_POINTER(static_cast<int>(factor * 10.0)));
        g_signal_connect(zb, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             const int z10 = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "zoom-x10"));
                             static_cast<CloseUpWritingPanel*>(selfPtr)->setZoomFactor(z10 / 10.0);
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(zoomSeg), zb, FALSE, FALSE, 0);
    }
    gtk_box_pack_start(GTK_BOX(header), zoomSeg, FALSE, FALSE, 6);

    GtkWidget* hintLbl = gtk_label_new("Giữ Shift + kéo để di chuyển khung");
    setCssClass(hintLbl, "swift-hint", true);
    gtk_box_pack_start(GTK_BOX(header), hintLbl, TRUE, TRUE, 4);

    // Auto-advance switch
    GtkWidget* aaLbl = gtk_label_new("Tự trượt");
    setCssClass(aaLbl, "swift-hint", true);
    GtkWidget* aaSw = gtk_switch_new();
    gtk_switch_set_active(GTK_SWITCH(aaSw), this->autoAdvance);
    g_signal_connect(aaSw, "state-set", G_CALLBACK(+[](GtkSwitch*, gboolean state, gpointer selfPtr) -> gboolean {
                         auto* self = static_cast<CloseUpWritingPanel*>(selfPtr);
                         self->autoAdvance = (state != FALSE);
                         self->shell->getSettingsMut().closeUpAutoAdvance = self->autoAdvance;
                         SwiftBridge::setSettings(self->shell->getSettings());
                         SwiftBridge::saveState();
                         self->queueRedraw();
                         return FALSE;
                     }),
                     this);
    gtk_box_pack_end(GTK_BOX(header), aaSw, FALSE, FALSE, 2);
    gtk_box_pack_end(GTK_BOX(header), aaLbl, FALSE, FALSE, 2);

    GtkWidget* btnClose = gtk_button_new_from_icon_name("window-close-symbolic", GTK_ICON_SIZE_MENU);
    gtk_button_set_relief(GTK_BUTTON(btnClose), GTK_RELIEF_NONE);
    gtk_widget_set_can_focus(btnClose, FALSE);
    gtk_widget_set_tooltip_text(btnClose, "Đóng vùng viết phóng to");
    g_signal_connect(btnClose, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<CloseUpWritingPanel*>(selfPtr)->setVisible(false);
                     }),
                     this);
    gtk_box_pack_end(GTK_BOX(header), btnClose, FALSE, FALSE, 0);

    gtk_box_pack_start(GTK_BOX(this->rootWidget), header, FALSE, FALSE, 0);

    // Zoomed writing drawing area
    this->drawingArea = gtk_drawing_area_new();
    const int panelH = static_cast<int>(std::clamp<uint32_t>(this->shell->getSettings().closeUpPanelHeightPx, 180, 480));
    gtk_widget_set_size_request(this->drawingArea, -1, panelH);
    gtk_widget_add_events(this->drawingArea,
                          GDK_BUTTON_PRESS_MASK | GDK_BUTTON_RELEASE_MASK | GDK_POINTER_MOTION_MASK |
                                  GDK_POINTER_MOTION_HINT_MASK);

    g_signal_connect(this->drawingArea, "draw", G_CALLBACK(onDraw), this);
    g_signal_connect(this->drawingArea, "button-press-event", G_CALLBACK(onButtonPress), this);
    g_signal_connect(this->drawingArea, "motion-notify-event", G_CALLBACK(onMotionNotify), this);
    g_signal_connect(this->drawingArea, "button-release-event", G_CALLBACK(onButtonRelease), this);

    gtk_box_pack_start(GTK_BOX(this->rootWidget), this->drawingArea, TRUE, TRUE, 0);
    gtk_widget_show_all(this->rootWidget);
    gtk_widget_hide(this->rootWidget);
}

XojPageView* CloseUpWritingPanel::currentPageView() const {
    if (!this->control || !this->control->getWindow() || !this->control->getWindow()->getXournal()) {
        return nullptr;
    }
    const size_t pageNr = this->control->getCurrentPageNo();
    if (pageNr == npos) {
        return nullptr;
    }
    return this->control->getWindow()->getXournal()->getViewFor(pageNr);
}

void CloseUpWritingPanel::clampFocusBox(double pageW, double pageH) {
    const int allocW = std::max(1, gtk_widget_get_allocated_width(this->drawingArea));
    const int allocH = std::max(1, gtk_widget_get_allocated_height(this->drawingArea));
    const double viewW = allocW / this->zoomFactor;
    const double viewH = allocH / this->zoomFactor;

    const double maxX = std::max(0.0, pageW - viewW * 0.5);
    const double maxY = std::max(0.0, pageH - viewH * 0.5);
    this->focusX = std::clamp(this->focusX, 0.0, maxX);
    this->focusY = std::clamp(this->focusY, 0.0, maxY);
}

void CloseUpWritingPanel::setVisible(bool show) {
    this->visible = show;
    if (show) {
        gtk_widget_show(this->rootWidget);
        queueRedraw();
    } else {
        cancelAutoAdvance();
        gtk_widget_hide(this->rootWidget);
    }
    if (this->shell) {
        this->shell->syncToolHighlight();
    }
}

void CloseUpWritingPanel::toggle() { setVisible(!this->visible); }

void CloseUpWritingPanel::moveLeft() {
    cancelAutoAdvance();
    const int allocW = std::max(1, gtk_widget_get_allocated_width(this->drawingArea));
    const double stepX = (allocW / this->zoomFactor) * 0.65;
    this->focusX = std::max(0.0, this->focusX - stepX);
    queueRedraw();
}

void CloseUpWritingPanel::moveRight() {
    cancelAutoAdvance();
    XojPageView* pv = currentPageView();
    if (!pv) {
        return;
    }
    const PageRef page = pv->getPage();
    const int allocW = std::max(1, gtk_widget_get_allocated_width(this->drawingArea));
    const double viewW = allocW / this->zoomFactor;
    const double stepX = viewW * 0.68;
    if (this->focusX + viewW >= page->getWidth() - 12.0) {
        nextLine();
    } else {
        this->focusX += stepX;
        clampFocusBox(page->getWidth(), page->getHeight());
        queueRedraw();
    }
}

void CloseUpWritingPanel::nextLine() {
    cancelAutoAdvance();
    XojPageView* pv = currentPageView();
    if (!pv) {
        return;
    }
    const PageRef page = pv->getPage();
    const int allocH = std::max(1, gtk_widget_get_allocated_height(this->drawingArea));
    const double lineStep = std::max(28.0, (allocH / this->zoomFactor) * 0.58);
    this->focusX = 36.0;
    this->focusY += lineStep;
    clampFocusBox(page->getWidth(), page->getHeight());
    queueRedraw();
}

void CloseUpWritingPanel::setZoomFactor(double factor) {
    this->zoomFactor = std::clamp(factor, 1.5, 5.0);
    this->shell->getSettingsMut().closeUpZoomFactor = this->zoomFactor;
    SwiftBridge::setSettings(this->shell->getSettings());
    SwiftBridge::saveState();
    queueRedraw();
}

void CloseUpWritingPanel::queueRedraw() {
    if (this->drawingArea && this->visible) {
        gtk_widget_queue_draw(this->drawingArea);
    }
}

void CloseUpWritingPanel::cancelAutoAdvance() {
    if (this->autoAdvanceTimerId != 0) {
        g_source_remove(this->autoAdvanceTimerId);
        this->autoAdvanceTimerId = 0;
    }
}

void CloseUpWritingPanel::scheduleAutoAdvance() {
    cancelAutoAdvance();
    this->autoAdvanceTimerId = g_timeout_add(360,
                                             +[](gpointer userData) -> gboolean {
                                                 auto* self = static_cast<CloseUpWritingPanel*>(userData);
                                                 self->autoAdvanceTimerId = 0;
                                                 if (self->visible && !self->strokeActive && self->autoAdvance) {
                                                     self->moveRight();
                                                 }
                                                 return G_SOURCE_REMOVE;
                                             },
                                             this);
}

gboolean CloseUpWritingPanel::onDraw(GtkWidget* widget, cairo_t* cr, gpointer userData) {
    auto* self = static_cast<CloseUpWritingPanel*>(userData);
    const int w = gtk_widget_get_allocated_width(widget);
    const int h = gtk_widget_get_allocated_height(widget);

    // Fill panel background cleanly (matte dark or light)
    if (DisplayState::isBlackboardActive()) {
        cairo_set_source_rgb(cr, 0.086, 0.094, 0.114);
    } else {
        cairo_set_source_rgb(cr, 0.95, 0.95, 0.97);
    }
    cairo_rectangle(cr, 0, 0, w, h);
    cairo_fill(cr);

    XojPageView* pv = self->currentPageView();
    if (!pv) {
        return TRUE;
    }

    const PageRef page = pv->getPage();
    self->clampFocusBox(page->getWidth(), page->getHeight());

    // Render page region at full vector crispness through Xournal++'s DocumentView
    cairo_save(cr);
    cairo_scale(cr, self->zoomFactor, self->zoomFactor);
    cairo_translate(cr, -self->focusX, -self->focusY);

    DocumentView docView;
    docView.setPdfCache(pv->getXournal()->getCache());
    docView.drawPage(page, cr, /*dontRenderEditingStroke=*/false);
    cairo_restore(cr);

    // Draw subtle right-edge auto-advance trigger band when enabled
    if (self->autoAdvance) {
        const double triggerX = w * self->marginTriggerRatio;
        cairo_save(cr);
        cairo_set_source_rgba(cr, 0.0, 0.48, 1.0, 0.07);
        cairo_rectangle(cr, triggerX, 0, w - triggerX, h);
        cairo_fill(cr);

        cairo_set_source_rgba(cr, 0.0, 0.48, 1.0, 0.35);
        cairo_set_line_width(cr, 1.0);
        const double dashes[] = {4.0, 4.0};
        cairo_set_dash(cr, dashes, 2, 0.0);
        cairo_move_to(cr, triggerX, 0);
        cairo_line_to(cr, triggerX, h);
        cairo_stroke(cr);
        cairo_restore(cr);
    }

    return TRUE;
}

gboolean CloseUpWritingPanel::onButtonPress(GtkWidget*, GdkEventButton* ev, gpointer userData) {
    auto* self = static_cast<CloseUpWritingPanel*>(userData);
    self->cancelAutoAdvance();

    if (ev->button == 2 || (ev->button == 1 && (ev->state & GDK_SHIFT_MASK))) {
        self->panningActive = true;
        self->panStartX = ev->x;
        self->panStartY = ev->y;
        self->panOriginFocusX = self->focusX;
        self->panOriginFocusY = self->focusY;
        return TRUE;
    }
    if (ev->button != 1) {
        return FALSE;
    }

    XojPageView* pv = self->currentPageView();
    if (!pv) {
        return FALSE;
    }

    const double pageX = self->focusX + ev->x / self->zoomFactor;
    const double pageY = self->focusY + ev->y / self->zoomFactor;
    const double pageZoom = pv->getZoom();

    PositionInputData pos{};
    pos.x = pageX * pageZoom;
    pos.y = pageY * pageZoom;
    pos.pressure = readEventPressure(reinterpret_cast<GdkEvent*>(ev));
    pos.timestamp = ev->time;
    pos.deviceId = DeviceId(gdk_event_get_source_device(reinterpret_cast<GdkEvent*>(ev)));
    pos.state = static_cast<GdkModifierType>(ev->state);

    self->strokeActive = true;
    self->maxStrokeWidgetX = ev->x;
    pv->onButtonPressEvent(pos);
    self->queueRedraw();
    return TRUE;
}

gboolean CloseUpWritingPanel::onMotionNotify(GtkWidget*, GdkEventMotion* ev, gpointer userData) {
    auto* self = static_cast<CloseUpWritingPanel*>(userData);
    gdk_event_request_motions(ev);

    if (self->panningActive) {
        const double dx = (ev->x - self->panStartX) / self->zoomFactor;
        const double dy = (ev->y - self->panStartY) / self->zoomFactor;
        self->focusX = self->panOriginFocusX - dx;
        self->focusY = self->panOriginFocusY - dy;
        if (auto* pv = self->currentPageView()) {
            const PageRef page = pv->getPage();
            self->clampFocusBox(page->getWidth(), page->getHeight());
        }
        self->queueRedraw();
        return TRUE;
    }

    if (!self->strokeActive) {
        return FALSE;
    }

    XojPageView* pv = self->currentPageView();
    if (!pv) {
        return FALSE;
    }

    const double pageX = self->focusX + ev->x / self->zoomFactor;
    const double pageY = self->focusY + ev->y / self->zoomFactor;
    const double pageZoom = pv->getZoom();

    PositionInputData pos{};
    pos.x = pageX * pageZoom;
    pos.y = pageY * pageZoom;
    pos.pressure = readEventPressure(reinterpret_cast<GdkEvent*>(ev));
    pos.timestamp = ev->time;
    pos.deviceId = DeviceId(gdk_event_get_source_device(reinterpret_cast<GdkEvent*>(ev)));
    pos.state = static_cast<GdkModifierType>(ev->state);

    self->maxStrokeWidgetX = std::max(self->maxStrokeWidgetX, ev->x);
    pv->onMotionNotifyEvent(pos);
    self->queueRedraw();
    return TRUE;
}

gboolean CloseUpWritingPanel::onButtonRelease(GtkWidget* widget, GdkEventButton* ev, gpointer userData) {
    auto* self = static_cast<CloseUpWritingPanel*>(userData);

    if (self->panningActive) {
        self->panningActive = false;
        return TRUE;
    }

    if (!self->strokeActive) {
        return FALSE;
    }
    self->strokeActive = false;

    XojPageView* pv = self->currentPageView();
    if (!pv) {
        return FALSE;
    }

    const double pageX = self->focusX + ev->x / self->zoomFactor;
    const double pageY = self->focusY + ev->y / self->zoomFactor;
    const double pageZoom = pv->getZoom();

    PositionInputData pos{};
    pos.x = pageX * pageZoom;
    pos.y = pageY * pageZoom;
    pos.pressure = readEventPressure(reinterpret_cast<GdkEvent*>(ev));
    pos.timestamp = ev->time;
    pos.deviceId = DeviceId(gdk_event_get_source_device(reinterpret_cast<GdkEvent*>(ev)));
    pos.state = static_cast<GdkModifierType>(ev->state);

    pv->onButtonReleaseEvent(pos);
    self->queueRedraw();

    // Auto-advance if stroke crossed into the right trigger zone while writing
    const ToolType curTool = self->control->getToolHandler()->getToolType();
    if (self->autoAdvance && (curTool == TOOL_PEN || curTool == TOOL_HIGHLIGHTER)) {
        const int w = gtk_widget_get_allocated_width(widget);
        if (w > 0 && self->maxStrokeWidgetX >= w * self->marginTriggerRatio) {
            self->scheduleAutoAdvance();
        }
    }
    return TRUE;
}

}  // namespace swiftnote

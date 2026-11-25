#include "SwiftShell.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstdio>

#include <cairo.h>
#include <gdk-pixbuf/gdk-pixbuf.h>
#include <gdk/gdkkeysyms.h>
#ifndef _WIN32
#include <fontconfig/fontconfig.h>
#endif

#include "control/Control.h"
#include "control/PageBackgroundChangeController.h"
#include "control/ToolHandler.h"
#include "control/actions/ActionDatabase.h"
#include "control/settings/Settings.h"
#include "control/settings/SettingsEnums.h"
#include "control/tools/TextEditor.h"
#include "enums/Action.enum.h"
#include "gui/GladeSearchpath.h"
#include "gui/MainWindow.h"
#include "gui/PageView.h"
#include "gui/XournalView.h"
#include "gui/swiftnote/CloseUpWritingPanel.h"
#include "gui/swiftnote/SwiftLibraryView.h"
#include "model/Document.h"
#include "model/Font.h"
#include "model/LineStyle.h"
#include "model/PageType.h"
#include "model/StrokeStyle.h"
#include "util/Color.h"
#include "util/StringUtils.h"
#include "util/Util.h"

#ifdef _WIN32
// Avoid pulling in <windows.h> macros; declare AddFontResourceExW directly for GDI/DirectWrite private font loading.
extern "C" __declspec(dllimport) int __stdcall AddFontResourceExW(const wchar_t* name, unsigned long fl, void* res);
static constexpr unsigned long SN_FR_PRIVATE = 0x10;
#endif

namespace swiftnote {

namespace {

constexpr std::array<uint32_t, 16> CURATED_INK_PALETTE = {
        0x111111,  // Matte Ink Black
        0xFFFFFF,  // Chalk White
        0x52525B,  // Slate Gray
        0x0A58CA,  // Cobalt Blue
        0x0071E3,  // Apple Blue
        0x0284C7,  // Sky Cyan
        0x0D9488,  // Teal
        0x15803D,  // Forest Green
        0x65A30D,  // Olive Green
        0xFACC15,  // Highlighter Yellow
        0xF97316,  // Warm Orange
        0xD92D20,  // Crimson Red
        0xE11D48,  // Rose
        0x9333EA,  // Purple
        0x4F46E5,  // Indigo
        0x854D0E,  // Espresso
};

void setCssClass(GtkWidget* widget, const char* cls, bool enabled) {
    if (!widget) {
        return;
    }
    GtkStyleContext* sc = gtk_widget_get_style_context(widget);
    if (enabled) {
        gtk_style_context_add_class(sc, cls);
    } else {
        gtk_style_context_remove_class(sc, cls);
    }
}

void colorToGdkRgba(Color c, GdkRGBA& out) {
    const uint32_t rgb = uint32_t(c) & 0x00FFFFFFU;
    out.red = ((rgb >> 16) & 0xFF) / 255.0;
    out.green = ((rgb >> 8) & 0xFF) / 255.0;
    out.blue = (rgb & 0xFF) / 255.0;
    out.alpha = 1.0;
}

Color gdkRgbaToColor(const GdkRGBA& c) {
    const uint32_t r = static_cast<uint32_t>(std::clamp(c.red * 255.0, 0.0, 255.0));
    const uint32_t g = static_cast<uint32_t>(std::clamp(c.green * 255.0, 0.0, 255.0));
    const uint32_t b = static_cast<uint32_t>(std::clamp(c.blue * 255.0, 0.0, 255.0));
    return Color{0xFF000000U | (r << 16) | (g << 8) | b};
}

std::string formatRgbCss(Color c) {
    char buf[16];
    std::snprintf(buf, sizeof(buf), "#%06X", uint32_t(c) & 0x00FFFFFFU);
    return buf;
}

std::string formatKeyUpper(const std::string& key) {
    if (key.empty()) {
        return "—";
    }
    if (key == "tab") {
        return "Tab";
    }
    if (key == "space") {
        return "Space";
    }
    std::string out = key;
    for (char& c: out) {
        c = static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
    }
    return out;
}

std::string normalizeGdkKeyval(guint keyval) {
    if (keyval == GDK_KEY_Tab || keyval == GDK_KEY_ISO_Left_Tab) {
        return "tab";
    }
    const guint lower = gdk_keyval_to_lower(keyval);
    if ((lower >= GDK_KEY_a && lower <= GDK_KEY_z) || (lower >= GDK_KEY_0 && lower <= GDK_KEY_9)) {
        return std::string(1, static_cast<char>(lower));
    }
    if (lower >= GDK_KEY_KP_0 && lower <= GDK_KEY_KP_9) {
        return std::string(1, static_cast<char>('0' + (lower - GDK_KEY_KP_0)));
    }
    return {};
}

LineStyle toXojLineStyle(SnLineStyle style) {
    switch (style) {
        case SN_LINE_STYLE_DASHED:
            return StrokeStyle::parseStyle("dash");
        case SN_LINE_STYLE_DOTTED:
            return StrokeStyle::parseStyle("dot");
        case SN_LINE_STYLE_SOLID:
        default:
            return StrokeStyle::parseStyle("plain");
    }
}

void drawRoundedRect(cairo_t* cr, double x, double y, double w, double h, double r) {
    const double rad = std::min(r, std::min(w, h) * 0.5);
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - rad, y + rad, rad, -G_PI * 0.5, 0.0);
    cairo_arc(cr, x + w - rad, y + h - rad, rad, 0.0, G_PI * 0.5);
    cairo_arc(cr, x + rad, y + h - rad, rad, G_PI * 0.5, G_PI);
    cairo_arc(cr, x + rad, y + rad, rad, G_PI, G_PI * 1.5);
    cairo_close_path(cr);
}

/**
 * Renders the exact approved A2 · Ink Stroke logo (swiftnote-logo-light.png / swiftnote-logo-dark.png)
 * with CAIRO_FILTER_BEST so it matches the approved artwork identically at every DPI.
 */
gboolean drawSwiftLogoMark(GtkWidget* widget, cairo_t* cr, gpointer userData) {
    auto* win = static_cast<MainWindow*>(userData);
    if (!win || !win->getGladeSearchPath()) {
        return FALSE;
    }
    const bool dark = win->isDarkTheme();
    static GdkPixbuf* cachedLight = nullptr;
    static GdkPixbuf* cachedDark = nullptr;
    GdkPixbuf*& slot = dark ? cachedDark : cachedLight;
    if (!slot) {
        const char* fname = dark ? "swiftnote-logo-dark.png" : "swiftnote-logo-light.png";
        auto p = win->getGladeSearchPath()->findFile("pixmaps", fname);
        if (!p.empty() && fs::exists(p)) {
            slot = gdk_pixbuf_new_from_file(char_cast(p.u8string().c_str()), nullptr);
        }
    }
    if (!slot) {
        return FALSE;
    }

    const int w = gtk_widget_get_allocated_width(widget);
    const int h = gtk_widget_get_allocated_height(widget);
    const double size = std::min(w, h);
    const double ox = (w - size) * 0.5;
    const double oy = (h - size) * 0.5;
    const int pw = gdk_pixbuf_get_width(slot);
    const int ph = gdk_pixbuf_get_height(slot);
    if (pw <= 0 || ph <= 0) {
        return FALSE;
    }

    cairo_save(cr);
    cairo_translate(cr, ox, oy);
    cairo_scale(cr, size / static_cast<double>(pw), size / static_cast<double>(ph));
    gdk_cairo_set_source_pixbuf(cr, slot, 0, 0);
    cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BEST);
    cairo_paint(cr);
    cairo_restore(cr);
    return FALSE;
}

struct PenIconData {
    uint32_t colorRgb;
    double widthPt;
    bool isHighlighter;
    bool active;
    MainWindow* win;
};

/**
 * Refined Apple-grade precision ink dot (14px solid circle with crisp outer ring when active).
 * Zero cartoon pen drawings, zero keycap text clutter.
 */
gboolean drawPenSlotIcon(GtkWidget* widget, cairo_t* cr, gpointer userData) {
    auto* data = static_cast<PenIconData*>(userData);
    const int w = gtk_widget_get_allocated_width(widget);
    const int h = gtk_widget_get_allocated_height(widget);
    const double cx = w * 0.5;
    const double cy = h * 0.5;
    const bool dark = data->win && data->win->isDarkTheme();

    const Color resolved = DisplayState::resolveInkColor(Color{data->colorRgb});
    const double r = ((uint32_t(resolved) >> 16) & 0xFF) / 255.0;
    const double g = ((uint32_t(resolved) >> 8) & 0xFF) / 255.0;
    const double b = (uint32_t(resolved) & 0xFF) / 255.0;

    if (data->isHighlighter) {
        const double bw = 15.0;
        const double bh = 9.0;
        if (data->active) {
            drawRoundedRect(cr, cx - bw * 0.5 - 3.0, cy - bh * 0.5 - 3.0, bw + 6.0, bh + 6.0, 5.5);
            if (dark) {
                cairo_set_source_rgb(cr, 0.90, 0.90, 0.94);
            } else {
                cairo_set_source_rgb(cr, 0.11, 0.11, 0.13);
            }
            cairo_set_line_width(cr, 1.4);
            cairo_stroke(cr);
        }
        drawRoundedRect(cr, cx - bw * 0.5, cy - bh * 0.5, bw, bh, 3.0);
        cairo_set_source_rgba(cr, r, g, b, 0.88);
        cairo_fill_preserve(cr);
        cairo_set_source_rgba(cr, 0.0, 0.0, 0.0, 0.16);
        cairo_set_line_width(cr, 0.8);
        cairo_stroke(cr);
    } else {
        const double dotR = data->active ? 6.4 : 6.2;
        if (data->active) {
            cairo_arc(cr, cx, cy, 9.6, 0, 2.0 * G_PI);
            if (dark) {
                cairo_set_source_rgb(cr, 0.92, 0.92, 0.95);
            } else {
                cairo_set_source_rgb(cr, 0.11, 0.11, 0.13);
            }
            cairo_set_line_width(cr, 1.5);
            cairo_stroke(cr);
        }
        cairo_arc(cr, cx, cy, dotR, 0, 2.0 * G_PI);
        cairo_set_source_rgb(cr, r, g, b);
        cairo_fill_preserve(cr);
        // Subtle 0.8px rim so pure white ink dot remains visible on white light-mode toolbar
        cairo_set_source_rgba(cr, dark ? 1.0 : 0.0, dark ? 1.0 : 0.0, dark ? 1.0 : 0.0, 0.16);
        cairo_set_line_width(cr, 0.8);
        cairo_stroke(cr);
    }
    return FALSE;
}

struct StrokePreviewData {
    uint32_t colorRgb;
    double widthPt;
    bool isHighlighter;
    SnLineStyle lineStyle;
};

gboolean drawStrokePreview(GtkWidget* widget, cairo_t* cr, gpointer userData) {
    auto* data = static_cast<StrokePreviewData*>(userData);
    const int w = gtk_widget_get_allocated_width(widget);
    const int h = gtk_widget_get_allocated_height(widget);

    drawRoundedRect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);
    const bool darkSurface = DisplayState::isBlackboardActive();
    if (darkSurface) {
        cairo_set_source_rgb(cr, 0.086, 0.094, 0.114);  // #16181D
    } else {
        cairo_set_source_rgb(cr, 0.965, 0.965, 0.975);
    }
    cairo_fill(cr);

    const Color resolved = DisplayState::resolveInkColor(Color{data->colorRgb});
    const double r = ((uint32_t(resolved) >> 16) & 0xFF) / 255.0;
    const double g = ((uint32_t(resolved) >> 8) & 0xFF) / 255.0;
    const double b = (uint32_t(resolved) & 0xFF) / 255.0;

    cairo_set_source_rgba(cr, r, g, b, data->isHighlighter ? 0.55 : 1.0);
    cairo_set_line_width(cr, std::clamp(data->widthPt * 1.4, 1.0, static_cast<double>(h) - 12.0));
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);

    if (data->lineStyle == SN_LINE_STYLE_DASHED) {
        const double dashes[] = {10.0, 6.0};
        cairo_set_dash(cr, dashes, 2, 0.0);
    } else if (data->lineStyle == SN_LINE_STYLE_DOTTED) {
        const double dashes[] = {1.5, 6.0};
        cairo_set_dash(cr, dashes, 2, 0.0);
    }

    const double pad = 18.0;
    const double midY = h * 0.5;
    cairo_move_to(cr, pad, midY + 4.0);
    cairo_curve_to(cr, w * 0.35, midY - 11.0, w * 0.65, midY + 11.0, w - pad, midY - 3.0);
    cairo_stroke(cr);
    return FALSE;
}

/**
 * Miniature vector wireframe diagrams inside Settings > Giao diện for B1, B2, B3 layouts.
 */
gboolean drawLayoutThumb(GtkWidget* widget, cairo_t* cr, gpointer userData) {
    const auto layout = static_cast<SnUiLayout>(GPOINTER_TO_INT(userData));
    const int w = gtk_widget_get_allocated_width(widget);
    const int h = gtk_widget_get_allocated_height(widget);

    // Frame canvas surround
    drawRoundedRect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 6.0);
    cairo_set_source_rgb(cr, 0.94, 0.94, 0.96);
    cairo_fill_preserve(cr);
    cairo_set_source_rgb(cr, 0.85, 0.85, 0.88);
    cairo_set_line_width(cr, 1.0);
    cairo_stroke(cr);

    // Page sheet
    const double pageTop = (layout == SN_LAYOUT_PURE_TOP_BAR) ? 20.0 : 18.0;
    const double pageH = (layout == SN_LAYOUT_ZEN_BOTTOM_DOCK) ? (h - 36.0) : (h - 24.0);
    drawRoundedRect(cr, 14.0, pageTop, w - 28.0, pageH, 3.0);
    cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    cairo_fill(cr);

    // Subtle ruled lines on mini page
    cairo_set_source_rgb(cr, 0.92, 0.92, 0.95);
    cairo_set_line_width(cr, 0.8);
    for (double y = pageTop + 9.0; y < pageTop + pageH - 5.0; y += 8.0) {
        cairo_move_to(cr, 20.0, y);
        cairo_line_to(cr, w - 20.0, y);
    }
    cairo_stroke(cr);

    // Layout-specific chrome wireframe
    if (layout == SN_LAYOUT_PURE_TOP_BAR) {
        cairo_rectangle(cr, 1.0, 1.0, w - 2.0, 13.0);
        cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
        cairo_fill(cr);
        cairo_set_source_rgb(cr, 0.84, 0.84, 0.88);
        cairo_move_to(cr, 1.0, 14.0);
        cairo_line_to(cr, w - 1.0, 14.0);
        cairo_stroke(cr);
        // Tiny ink dots centered
        for (int i = 0; i < 4; ++i) {
            cairo_arc(cr, w * 0.5 - 12.0 + i * 8.0, 7.5, 2.2, 0, 2.0 * G_PI);
            cairo_set_source_rgb(cr, i == 0 ? 0.1 : 0.0, i == 1 ? 0.44 : 0.25, i == 1 ? 0.89 : 0.35);
            cairo_fill(cr);
        }
    } else if (layout == SN_LAYOUT_CANVAS_PILL) {
        // 3 slim floating pills tight at top
        drawRoundedRect(cr, 6.0, 4.0, 24.0, 10.0, 5.0);
        drawRoundedRect(cr, w * 0.5 - 24.0, 4.0, 48.0, 10.0, 5.0);
        drawRoundedRect(cr, w - 30.0, 4.0, 24.0, 10.0, 5.0);
        cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
        cairo_fill_preserve(cr);
        cairo_set_source_rgb(cr, 0.82, 0.82, 0.86);
        cairo_set_line_width(cr, 0.8);
        cairo_stroke(cr);
        for (int i = 0; i < 4; ++i) {
            cairo_arc(cr, w * 0.5 - 12.0 + i * 8.0, 9.0, 2.0, 0, 2.0 * G_PI);
            cairo_set_source_rgb(cr, i == 0 ? 0.1 : 0.0, i == 1 ? 0.44 : 0.3, i == 1 ? 0.89 : 0.4);
            cairo_fill(cr);
        }
    } else {
        // Zen quiet top + bottom centered Ink Dock
        cairo_set_source_rgb(cr, 0.82, 0.82, 0.86);
        cairo_rectangle(cr, 8.0, 5.5, 22.0, 3.0);
        cairo_rectangle(cr, w - 30.0, 5.5, 22.0, 3.0);
        cairo_fill(cr);

        drawRoundedRect(cr, w * 0.5 - 30.0, h - 15.0, 60.0, 11.0, 5.5);
        cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
        cairo_fill_preserve(cr);
        cairo_set_source_rgb(cr, 0.80, 0.80, 0.85);
        cairo_set_line_width(cr, 0.8);
        cairo_stroke(cr);
        for (int i = 0; i < 4; ++i) {
            cairo_arc(cr, w * 0.5 - 14.0 + i * 8.0, h - 9.5, 2.1, 0, 2.0 * G_PI);
            cairo_set_source_rgb(cr, i == 0 ? 0.1 : 0.0, i == 1 ? 0.44 : 0.3, i == 1 ? 0.89 : 0.4);
            cairo_fill(cr);
        }
    }
    return FALSE;
}

/**
 * Live Blackboard + Auto Ink Inversion preview inside Settings > Bảng đen & Giấy.
 */
gboolean drawBlackboardLivePreview(GtkWidget* widget, cairo_t* cr, gpointer) {
    const int w = gtk_widget_get_allocated_width(widget);
    const int h = gtk_widget_get_allocated_height(widget);
    const bool bb = DisplayState::isBlackboardActive();

    drawRoundedRect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 10.0);
    if (bb) {
        cairo_set_source_rgb(cr, 0.086, 0.094, 0.114);  // #16181D
    } else {
        cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    }
    cairo_fill_preserve(cr);
    cairo_set_source_rgb(cr, bb ? 0.22 : 0.86, bb ? 0.24 : 0.86, bb ? 0.28 : 0.89);
    cairo_set_line_width(cr, 1.0);
    cairo_stroke(cr);

    // Subtle ruled baseline
    cairo_set_source_rgba(cr, bb ? 1.0 : 0.0, bb ? 1.0 : 0.0, bb ? 1.0 : 0.0, 0.08);
    cairo_set_line_width(cr, 1.0);
    cairo_move_to(cr, 16, h * 0.65);
    cairo_line_to(cr, w - 16, h * 0.65);
    cairo_stroke(cr);

    const std::array<std::pair<Color, const char*>, 4> samples = {{
            {Color{0x111111U}, "Mực đen"},
            {Color{0x0071E3U}, "Xanh dương"},
            {Color{0xD92D20U}, "Mực đỏ"},
            {Color{0xFACC15U}, "Dạ quang"},
    }};
    const double colW = (w - 32.0) / 4.0;
    for (size_t i = 0; i < samples.size(); ++i) {
        const double x0 = 16.0 + static_cast<double>(i) * colW + 8.0;
        const double x1 = 16.0 + static_cast<double>(i + 1) * colW - 12.0;
        const Color c = DisplayState::resolveInkColor(samples[i].first);
        const double r = ((uint32_t(c) >> 16) & 0xFF) / 255.0;
        const double g = ((uint32_t(c) >> 8) & 0xFF) / 255.0;
        const double b = (uint32_t(c) & 0xFF) / 255.0;

        cairo_set_source_rgba(cr, r, g, b, i == 3 ? 0.65 : 1.0);
        cairo_set_line_width(cr, i == 3 ? 9.0 : 2.6);
        cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
        cairo_move_to(cr, x0, h * 0.56);
        cairo_curve_to(cr, x0 + (x1 - x0) * 0.35, h * 0.25, x0 + (x1 - x0) * 0.65, h * 0.72, x1, h * 0.44);
        cairo_stroke(cr);
    }
    return FALSE;
}

GtkWidget* makeVerticalDivider() {
    GtkWidget* sep = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_style_context_add_class(gtk_widget_get_style_context(sep), "swift-divider");
    gtk_widget_set_size_request(sep, 1, 18);
    return sep;
}

}  // namespace

SwiftShell::SwiftShell(MainWindow* win, Control* control): win(win), control(control) {
    SwiftBridge::initState();
    this->settings = SwiftBridge::getSettings();
    this->penSlots = SwiftBridge::getPenSlots();
    this->shortcuts = SwiftBridge::getShortcuts();
    this->lastWritingSlot = this->settings.activePenSlot;

    registerEraseFinishedCallback([this]() { this->onEraseStrokeFinished(); });
}

SwiftShell::~SwiftShell() {
    registerEraseFinishedCallback(nullptr);
    if (this->activePopover) {
        g_signal_handlers_disconnect_by_data(this->activePopover, this);
        gtk_widget_destroy(this->activePopover);
        this->activePopover = nullptr;
    }
    if (this->leftCluster) {
        g_object_unref(this->leftCluster);
    }
    if (this->centerCluster) {
        g_object_unref(this->centerCluster);
    }
    if (this->rightCluster) {
        g_object_unref(this->rightCluster);
    }
}

void SwiftShell::loadBundledFonts() {
    if (!this->win || !this->win->getGladeSearchPath()) {
        return;
    }
    auto fontPath = this->win->getGladeSearchPath()->findFile("fonts", "PlusJakartaSans.ttf");
    if (fontPath.empty() || !fs::exists(fontPath)) {
        return;
    }
#ifdef _WIN32
    AddFontResourceExW(fontPath.wstring().c_str(), SN_FR_PRIVATE, nullptr);
#else
    FcConfigAppFontAddFile(nullptr, reinterpret_cast<const FcChar8*>(fontPath.c_str()));
#endif
}

void SwiftShell::hideLegacyChrome() {
    if (!this->win) {
        return;
    }
    // Keep legacy Xournal++ statusbar ("State PLACEHOLDER") and searchBar hidden unless explicitly activated
    if (GtkWidget* sb = this->win->get("statusbar")) {
        gtk_widget_hide(sb);
    }
    if (GtkWidget* searchBar = this->win->get("searchBar")) {
        gtk_widget_hide(searchBar);
    }
    if (GtkWidget* searchToolbar = this->win->get("searchToolbar")) {
        gtk_widget_hide(searchToolbar);
    }
}

void SwiftShell::mount(GtkOverlay* mainOverlay, GtkWidget* mainBox) {
    if (!mainOverlay || !mainBox) {
        return;
    }

    loadBundledFonts();

    if (control && control->getSettings()) {
        control->getSettings()->setIconTheme(ICON_THEME_LUCIDE);
    }

    // Detach mainBox from mainOverlay without destroying it
    g_object_ref(mainBox);
    gtk_container_remove(GTK_CONTAINER(mainOverlay), mainBox);

    this->stack = gtk_stack_new();
    gtk_stack_set_transition_type(GTK_STACK(this->stack), GTK_STACK_TRANSITION_TYPE_CROSSFADE);
    gtk_stack_set_transition_duration(GTK_STACK(this->stack), 140);

    this->editorShell = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);

    this->topHost = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_name(this->topHost, "swiftTopHost");

    this->bottomHost = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_name(this->bottomHost, "swiftBottomHost");

    buildToolClusters();

    gtk_box_pack_start(GTK_BOX(this->editorShell), this->topHost, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(this->editorShell), mainBox, TRUE, TRUE, 0);
    g_object_unref(mainBox);

    this->closeUpPanel = std::make_unique<CloseUpWritingPanel>(this, this->control);
    gtk_box_pack_end(GTK_BOX(this->editorShell), this->bottomHost, FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(this->editorShell), this->closeUpPanel->getWidget(), FALSE, FALSE, 0);

    this->libraryView = std::make_unique<SwiftLibraryView>(this, this->control);

    gtk_stack_add_named(GTK_STACK(this->stack), this->libraryView->getWidget(), "library");
    gtk_stack_add_named(GTK_STACK(this->stack), this->editorShell, "editor");
    gtk_stack_set_visible_child_name(GTK_STACK(this->stack), "editor");

    gtk_container_add(GTK_CONTAINER(mainOverlay), this->stack);
    // IMPORTANT: Never call gtk_widget_show_all(this->stack), because mainBox contains hidden legacy
    // Glade widgets (statusbar "State PLACEHOLDER", searchToolbar, etc.) that must stay hidden!
    gtk_widget_show_all(this->libraryView->getWidget());
    gtk_widget_show(mainBox);
    gtk_widget_show(this->editorShell);
    gtk_widget_show(this->stack);
    this->closeUpPanel->setVisible(false);
    hideLegacyChrome();

    // Mount the chosen layout (B1 / B2 / B3) cleanly
    applyLayoutMode(this->settings.uiLayout);

    // Connect Scrble Ink single-key shortcut handler on mainWindow
    g_signal_connect(this->win->getWindow(), "key-press-event",
                     G_CALLBACK(+[](GtkWidget*, GdkEventKey* ev, gpointer selfPtr) -> gboolean {
                         return static_cast<SwiftShell*>(selfPtr)->handleKeyPress(ev) ? TRUE : FALSE;
                     }),
                     this);

    // Defer tool/theme sync until Control::initWindow() has constructed ActionDatabase & Document
    Util::execInUiThread([this]() {
        this->hideLegacyChrome();
        this->applyThemeFromSettings();
        if (!this->penSlots.empty()) {
            this->selectPenSlot(
                    std::min<uint32_t>(this->settings.activePenSlot, static_cast<uint32_t>(this->penSlots.size() - 1)));
        }
        this->refreshTooltipsWithShortcuts();
        this->syncToolHighlight();
    });
}

GtkWidget* SwiftShell::makeIconButton(const char* iconName, const char* tooltip) {
    GtkWidget* btn = gtk_button_new();
    gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
    gtk_widget_set_can_focus(btn, FALSE);
    GtkWidget* img = gtk_image_new_from_icon_name(iconName, GTK_ICON_SIZE_MENU);
    gtk_image_set_pixel_size(GTK_IMAGE(img), 16);
    gtk_container_add(GTK_CONTAINER(btn), img);
    if (tooltip) {
        gtk_widget_set_tooltip_text(btn, tooltip);
    }
    return btn;
}

GtkWidget* SwiftShell::makeMenuRow(const char* iconName, const char* labelText, bool active) {
    GtkWidget* btn = gtk_button_new();
    gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
    gtk_widget_set_can_focus(btn, FALSE);
    setCssClass(btn, "swift-menu-row", true);
    if (active) {
        setCssClass(btn, "swift-active", true);
    }

    GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    if (iconName && iconName[0] != '\0') {
        GtkWidget* img = gtk_image_new_from_icon_name(iconName, GTK_ICON_SIZE_MENU);
        gtk_image_set_pixel_size(GTK_IMAGE(img), 15);
        gtk_box_pack_start(GTK_BOX(row), img, FALSE, FALSE, 0);
    }
    GtkWidget* lbl = gtk_label_new(labelText);
    gtk_label_set_xalign(GTK_LABEL(lbl), 0.0f);
    gtk_box_pack_start(GTK_BOX(row), lbl, TRUE, TRUE, 0);
    gtk_container_add(GTK_CONTAINER(btn), row);
    return btn;
}

void SwiftShell::closeActivePopover() {
    if (this->activePopover) {
        GtkWidget* pop = this->activePopover;
        this->activePopover = nullptr;
        gtk_popover_popdown(GTK_POPOVER(pop));
    }
}

GtkWidget* SwiftShell::makePopover(GtkWidget* anchor) {
    closeActivePopover();
    GtkWidget* pop = gtk_popover_new(anchor);
    const GtkPositionType pos = (this->settings.uiLayout == SN_LAYOUT_ZEN_BOTTOM_DOCK &&
                                 gtk_widget_is_ancestor(anchor, this->bottomHost))
                                        ? GTK_POS_TOP
                                        : GTK_POS_BOTTOM;
    gtk_popover_set_position(GTK_POPOVER(pop), pos);
    setCssClass(pop, "swift-popover", true);
    if (win && win->isDarkTheme()) {
        setCssClass(pop, "darkMode", true);
    }
    g_signal_connect(pop, "closed", G_CALLBACK(+[](GtkPopover* p, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         GtkWidget* w = GTK_WIDGET(p);
                         if (self->activePopover == w) {
                             self->activePopover = nullptr;
                         }
                         if (self->win && self->win->getXournal()) {
                             self->win->getXournal()->requestFocus();
                         }
                         if (!g_object_get_data(G_OBJECT(w), "swift-destroy-queued")) {
                             g_object_set_data(G_OBJECT(w), "swift-destroy-queued", GINT_TO_POINTER(1));
                             g_object_ref(w);
                             g_idle_add(
                                     +[](gpointer data) -> gboolean {
                                         GtkWidget* widget = GTK_WIDGET(data);
                                         if (GTK_IS_WIDGET(widget)) {
                                             gtk_widget_destroy(widget);
                                         }
                                         g_object_unref(widget);
                                         return G_SOURCE_REMOVE;
                                     },
                                     w);
                         }
                     }),
                     this);
    this->activePopover = pop;
    return pop;
}

void SwiftShell::buildToolClusters() {
    // === LEFT CLUSTER: A2 Logo (Library) + Document Title (File Menu) ===
    this->leftCluster = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    g_object_ref_sink(this->leftCluster);
    setCssClass(this->leftCluster, "swift-cluster", true);

    this->btnLibrary = gtk_button_new();
    gtk_button_set_relief(GTK_BUTTON(this->btnLibrary), GTK_RELIEF_NONE);
    gtk_widget_set_can_focus(this->btnLibrary, FALSE);
    gtk_widget_set_tooltip_text(this->btnLibrary, "Thư viện Swift note");
    this->logoArea = gtk_drawing_area_new();
    gtk_widget_set_size_request(this->logoArea, 22, 22);
    g_signal_connect(this->logoArea, "draw", G_CALLBACK(drawSwiftLogoMark), this->win);
    gtk_container_add(GTK_CONTAINER(this->btnLibrary), this->logoArea);
    g_signal_connect(this->btnLibrary, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->showLibrary();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->leftCluster), this->btnLibrary, FALSE, FALSE, 0);

    GtkWidget* titleBtn = gtk_button_new();
    gtk_button_set_relief(GTK_BUTTON(titleBtn), GTK_RELIEF_NONE);
    gtk_widget_set_can_focus(titleBtn, FALSE);
    gtk_widget_set_tooltip_text(titleBtn, "Tệp, Mở PDF, Xuất & Mẫu giấy");
    GtkWidget* titleRow = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    this->docTitleLabel = gtk_label_new("Ghi chú mới");
    gtk_label_set_ellipsize(GTK_LABEL(this->docTitleLabel), PANGO_ELLIPSIZE_END);
    gtk_label_set_max_width_chars(GTK_LABEL(this->docTitleLabel), 20);
    setCssClass(this->docTitleLabel, "swift-doc-title", true);
    gtk_box_pack_start(GTK_BOX(titleRow), this->docTitleLabel, FALSE, FALSE, 0);
    GtkWidget* chevron = gtk_image_new_from_icon_name("pan-down-symbolic", GTK_ICON_SIZE_MENU);
    gtk_image_set_pixel_size(GTK_IMAGE(chevron), 12);
    gtk_box_pack_start(GTK_BOX(titleRow), chevron, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(titleBtn), titleRow);
    g_signal_connect(titleBtn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->openFileMenuPopover(GTK_WIDGET(b));
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->leftCluster), titleBtn, FALSE, FALSE, 0);

    // === CENTER CLUSTER: Precision Ink Dots + Quiet Monochrome Writing Tools ===
    this->centerCluster = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 3);
    g_object_ref_sink(this->centerCluster);
    setCssClass(this->centerCluster, "swift-cluster", true);
    setCssClass(this->centerCluster, "swift-cluster-center", true);

    this->penSlotsBox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 2);
    rebuildPenSlotButtons();
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->penSlotsBox, FALSE, FALSE, 0);

    gtk_box_pack_start(GTK_BOX(this->centerCluster), makeVerticalDivider(), FALSE, FALSE, 0);

    this->btnEraser = makeIconButton("xopp-tool-eraser", "Tẩy");
    g_signal_connect(this->btnEraser, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         if (self->control->getToolHandler()->getToolType() == TOOL_ERASER) {
                             self->openEraserPopover(GTK_WIDGET(b));
                         } else {
                             self->control->selectTool(TOOL_ERASER);
                             self->syncToolHighlight();
                         }
                     }),
                     this);
    g_signal_connect(this->btnEraser, "button-press-event",
                     G_CALLBACK(+[](GtkWidget* w, GdkEventButton* ev, gpointer selfPtr) -> gboolean {
                         if (ev->button == 3) {
                             static_cast<SwiftShell*>(selfPtr)->openEraserPopover(w);
                             return TRUE;
                         }
                         return FALSE;
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnEraser, FALSE, FALSE, 0);

    this->btnSelection = makeIconButton("xopp-select-lasso", "Chọn vùng");
    g_signal_connect(this->btnSelection, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         const auto t = self->control->getToolHandler()->getToolType();
                         if (t == TOOL_SELECT_REGION || t == TOOL_SELECT_RECT || t == TOOL_SELECT_OBJECT ||
                             t == TOOL_SELECT_PDF_TEXT_LINEAR || t == TOOL_SELECT_PDF_TEXT_RECT) {
                             self->openSelectionPopover(GTK_WIDGET(b));
                         } else {
                             self->control->selectTool(TOOL_SELECT_REGION);
                             self->syncToolHighlight();
                         }
                     }),
                     this);
    g_signal_connect(this->btnSelection, "button-press-event",
                     G_CALLBACK(+[](GtkWidget* w, GdkEventButton* ev, gpointer selfPtr) -> gboolean {
                         if (ev->button == 3) {
                             static_cast<SwiftShell*>(selfPtr)->openSelectionPopover(w);
                             return TRUE;
                         }
                         return FALSE;
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnSelection, FALSE, FALSE, 0);

    this->btnShapes = makeIconButton("xopp-shape-recognizer", "Hình học & Ink-to-Shape");
    g_signal_connect(this->btnShapes, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->openShapesPopover(GTK_WIDGET(b));
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnShapes, FALSE, FALSE, 0);

    this->btnText = makeIconButton("xopp-tool-text", "Văn bản (Text Box)");
    g_signal_connect(this->btnText, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         if (self->control->getToolHandler()->getToolType() == TOOL_TEXT) {
                             self->openTextPopover(GTK_WIDGET(b));
                         } else {
                             self->control->selectTool(TOOL_TEXT);
                             if (self->win && self->win->getXournal()) {
                                 self->win->getXournal()->requestFocus();
                             }
                             self->syncToolHighlight();
                         }
                     }),
                     this);
    g_signal_connect(this->btnText, "button-press-event",
                     G_CALLBACK(+[](GtkWidget* w, GdkEventButton* ev, gpointer selfPtr) -> gboolean {
                         if (ev->button == 3) {
                             static_cast<SwiftShell*>(selfPtr)->openTextPopover(w);
                             return TRUE;
                         }
                         return FALSE;
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnText, FALSE, FALSE, 0);

    this->btnRuler = makeIconButton("xopp-setsquare", "Thước ê-ke & Compa");
    g_signal_connect(this->btnRuler, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->openRulerPopover(GTK_WIDGET(b));
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnRuler, FALSE, FALSE, 0);

    this->btnSpacer = makeIconButton("xopp-spacer", "Dịch khoảng trống dọc");
    g_signal_connect(this->btnSpacer, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         self->control->selectTool(TOOL_VERTICAL_SPACE);
                         self->syncToolHighlight();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnSpacer, FALSE, FALSE, 0);

    this->btnLaserHand = makeIconButton("xopp-laser-pointer", "Con trỏ Laser & Cuộn tay");
    g_signal_connect(this->btnLaserHand, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->openLaserHandPopover(GTK_WIDGET(b));
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->centerCluster), this->btnLaserHand, FALSE, FALSE, 0);

    // === RIGHT CLUSTER: Undo/Redo, Blackboard, Close-up, Page Counter, Sidebar, Settings ===
    this->rightCluster = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 2);
    g_object_ref_sink(this->rightCluster);
    setCssClass(this->rightCluster, "swift-cluster", true);

    this->btnUndo = makeIconButton("xopp-edit-undo", "Hoàn tác (Ctrl+Z)");
    g_signal_connect(this->btnUndo, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->control->getActionDatabase()->fireActivateAction(
                                 Action::UNDO);
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), this->btnUndo, FALSE, FALSE, 0);

    this->btnRedo = makeIconButton("xopp-edit-redo", "Làm lại (Ctrl+Y)");
    g_signal_connect(this->btnRedo, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->control->getActionDatabase()->fireActivateAction(
                                 Action::REDO);
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), this->btnRedo, FALSE, FALSE, 0);

    gtk_box_pack_start(GTK_BOX(this->rightCluster), makeVerticalDivider(), FALSE, FALSE, 0);

    // Quiet monochrome Blackboard icon button (no noisy text pill)
    this->btnBlackboard = makeIconButton("xopp-presentation-mode", "Chế độ Bảng đen");
    g_signal_connect(this->btnBlackboard, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->toggleBlackboardMode();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), this->btnBlackboard, FALSE, FALSE, 0);

    this->btnCloseUp = makeIconButton("zoom-in-symbolic", "Vùng viết phóng to (Close-up)");
    g_signal_connect(this->btnCloseUp, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->toggleCloseUpPanel();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), this->btnCloseUp, FALSE, FALSE, 0);

    GtkWidget* pagePill = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
    setCssClass(pagePill, "swift-page-pill", true);
    this->pageInfoLabel = gtk_label_new("1 / 1");
    gtk_box_pack_start(GTK_BOX(pagePill), this->pageInfoLabel, FALSE, FALSE, 2);
    this->btnAddPage = makeIconButton("xopp-page-add", "Thêm trang mới");
    g_signal_connect(this->btnAddPage, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->control->getActionDatabase()->fireActivateAction(
                                 Action::NEW_PAGE_AFTER);
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(pagePill), this->btnAddPage, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), pagePill, FALSE, FALSE, 1);

    this->btnSidebar = makeIconButton("xopp-sidebar-show", "Danh sách trang");
    g_signal_connect(this->btnSidebar, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         const bool next = !self->control->getSettings()->isSidebarVisible();
                         self->control->setShowSidebar(next);
                         self->syncToolHighlight();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), this->btnSidebar, FALSE, FALSE, 0);

    this->btnSettings = makeIconButton("emblem-system-symbolic", "Cài đặt Swift note & Phím tắt");
    g_signal_connect(this->btnSettings, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftShell*>(selfPtr)->openSettingsDialog();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(this->rightCluster), this->btnSettings, FALSE, FALSE, 0);
}

void SwiftShell::applyLayoutMode(SnUiLayout layout) {
    this->settings.uiLayout = layout;
    if (!this->win || !this->topHost || !this->bottomHost) {
        return;
    }

    GtkWidget* windowWidget = this->win->getWindow();
    setCssClass(windowWidget, "layout-b1", layout == SN_LAYOUT_PURE_TOP_BAR);
    setCssClass(windowWidget, "layout-b2", layout == SN_LAYOUT_CANVAS_PILL);
    setCssClass(windowWidget, "layout-b3", layout == SN_LAYOUT_ZEN_BOTTOM_DOCK);

    // Detach all 3 clusters safely (they are ref'd by SwiftShell)
    auto detach = [](GtkWidget* w) {
        if (!w) {
            return;
        }
        if (GtkWidget* parent = gtk_widget_get_parent(w)) {
            gtk_container_remove(GTK_CONTAINER(parent), w);
        }
    };
    detach(this->leftCluster);
    detach(this->centerCluster);
    detach(this->rightCluster);
    gtk_box_set_center_widget(GTK_BOX(this->topHost), nullptr);
    gtk_box_set_center_widget(GTK_BOX(this->bottomHost), nullptr);

    // Reparent clusters cleanly for the active layout
    gtk_box_pack_start(GTK_BOX(this->topHost), this->leftCluster, FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(this->topHost), this->rightCluster, FALSE, FALSE, 0);

    if (layout == SN_LAYOUT_ZEN_BOTTOM_DOCK) {
        gtk_box_set_center_widget(GTK_BOX(this->bottomHost), this->centerCluster);
        gtk_widget_show_all(this->topHost);
        gtk_widget_show_all(this->bottomHost);
        if (this->focusModeHidden) {
            gtk_widget_hide(this->topHost);
            gtk_widget_hide(this->bottomHost);
        }
    } else {
        gtk_box_set_center_widget(GTK_BOX(this->topHost), this->centerCluster);
        gtk_widget_show_all(this->topHost);
        gtk_widget_hide(this->bottomHost);
        if (this->focusModeHidden) {
            gtk_widget_hide(this->topHost);
        }
    }
}

void SwiftShell::toggleFocusMode() {
    this->focusModeHidden = !this->focusModeHidden;
    if (this->focusModeHidden) {
        gtk_widget_hide(this->topHost);
        gtk_widget_hide(this->bottomHost);
    } else {
        applyLayoutMode(this->settings.uiLayout);
    }
}

std::string SwiftShell::getShortcutKeyDisplay(const std::string& actionId) const {
    for (const auto& sc: this->shortcuts) {
        if (sc.actionId == actionId) {
            return formatKeyUpper(sc.key);
        }
    }
    return {};
}

void SwiftShell::refreshTooltipsWithShortcuts() {
    auto tip = [this](const char* name, const char* actionId, const char* extra = nullptr) -> std::string {
        std::string res = name;
        const std::string k = getShortcutKeyDisplay(actionId);
        if (!k.empty() && k != "—") {
            res += "  [" + k + "]";
        }
        if (extra) {
            res += std::string("  •  ") + extra;
        }
        return res;
    };

    if (this->btnEraser) {
        gtk_widget_set_tooltip_text(this->btnEraser, tip("Tẩy", "eraser", "bấm lại để chỉnh").c_str());
    }
    if (this->btnSelection) {
        gtk_widget_set_tooltip_text(this->btnSelection, tip("Chọn vùng (Lasso / Box)", "lasso").c_str());
    }
    if (this->btnShapes) {
        gtk_widget_set_tooltip_text(this->btnShapes, tip("Hình học & Ink-to-Shape", "shapes").c_str());
    }
    if (this->btnText) {
        gtk_widget_set_tooltip_text(this->btnText, tip("Văn bản (Text Box)", "text").c_str());
    }
    if (this->btnRuler) {
        gtk_widget_set_tooltip_text(this->btnRuler, tip("Thước ê-ke & Compa", "ruler").c_str());
    }
    if (this->btnSpacer) {
        gtk_widget_set_tooltip_text(this->btnSpacer, tip("Dịch khoảng trống dọc", "spacer").c_str());
    }
    if (this->btnLaserHand) {
        gtk_widget_set_tooltip_text(this->btnLaserHand, tip("Laser & Chế độ cuộn tay", "laser").c_str());
    }
    if (this->btnBlackboard) {
        gtk_widget_set_tooltip_text(this->btnBlackboard, tip("Chế độ Bảng đen", "blackboard").c_str());
    }
    if (this->btnCloseUp) {
        gtk_widget_set_tooltip_text(this->btnCloseUp, tip("Vùng viết phóng to (Close-up)", "close_up").c_str());
    }
    if (this->btnAddPage) {
        gtk_widget_set_tooltip_text(this->btnAddPage, tip("Thêm trang mới", "new_page").c_str());
    }
}

bool SwiftShell::handleKeyPress(GdkEventKey* ev) {
    if (!ev || !this->control || !this->win || !this->stack) {
        return false;
    }
    // Only dispatch when the editor canvas is visible
    const char* visiblePage = gtk_stack_get_visible_child_name(GTK_STACK(this->stack));
    if (!visiblePage || std::string(visiblePage) != "editor") {
        return false;
    }
    // Never steal modified shortcuts (Ctrl+Z, Ctrl+V, Alt+..., etc.)
    const guint blockedMods = GDK_CONTROL_MASK | GDK_MOD1_MASK | GDK_SUPER_MASK | GDK_META_MASK;
    if ((ev->state & blockedMods) != 0) {
        return false;
    }
    // Never steal keystrokes while a popover is open
    if (this->activePopover != nullptr) {
        return false;
    }
    // Never steal keystrokes while typing in a Text Box (Vietnamese IME)
    if (this->win->getXournal() && this->win->getXournal()->getTextEditor() != nullptr) {
        return false;
    }
    // Never steal keystrokes when any text input/search widget has keyboard focus
    if (GtkWidget* focused = gtk_window_get_focus(GTK_WINDOW(this->win->getWindow()))) {
        if (GTK_IS_EDITABLE(focused) || GTK_IS_TEXT_VIEW(focused)) {
            return false;
        }
    }

    const std::string key = normalizeGdkKeyval(ev->keyval);
    if (key.empty()) {
        return false;
    }

    for (const auto& sc: this->shortcuts) {
        if (!sc.key.empty() && sc.key == key) {
            dispatchShortcutAction(sc.actionId);
            return true;
        }
    }
    // Extra convenience: digits 6..8 select pen slots 6..8 if present
    if (key.size() == 1 && key[0] >= '6' && key[0] <= '8') {
        const uint32_t idx = static_cast<uint32_t>(key[0] - '1');
        if (idx < this->penSlots.size()) {
            selectPenSlot(idx);
            return true;
        }
    }
    return false;
}

void SwiftShell::dispatchShortcutAction(const std::string& actionId) {
    if (actionId.rfind("pen_", 0) == 0 && actionId.size() == 5) {
        const uint32_t idx = static_cast<uint32_t>(actionId[4] - '1');
        if (idx < this->penSlots.size()) {
            selectPenSlot(idx);
        }
        return;
    }
    if (actionId == "eraser") {
        if (this->control->getToolHandler()->getToolType() == TOOL_ERASER &&
            this->lastWritingSlot < this->penSlots.size()) {
            selectPenSlot(this->lastWritingSlot);
        } else {
            this->control->selectTool(TOOL_ERASER);
            syncToolHighlight();
        }
    } else if (actionId == "lasso") {
        this->control->selectTool(TOOL_SELECT_REGION);
        syncToolHighlight();
    } else if (actionId == "box_select") {
        this->control->selectTool(TOOL_SELECT_RECT);
        syncToolHighlight();
    } else if (actionId == "shapes") {
        auto* th = this->control->getToolHandler();
        if (th->getToolType() != TOOL_PEN && th->getToolType() != TOOL_HIGHLIGHTER) {
            this->control->selectTool(TOOL_PEN);
        }
        const bool next = (th->getDrawingType() != DRAWING_TYPE_SHAPE_RECOGNIZER);
        th->setDrawingType(next ? DRAWING_TYPE_SHAPE_RECOGNIZER : DRAWING_TYPE_DEFAULT);
        syncToolHighlight();
    } else if (actionId == "text") {
        this->control->selectTool(TOOL_TEXT);
        if (this->win && this->win->getXournal()) {
            this->win->getXournal()->requestFocus();
        }
        syncToolHighlight();
    } else if (actionId == "ruler") {
        this->control->getActionDatabase()->fireActivateAction(Action::SETSQUARE);
        syncToolHighlight();
    } else if (actionId == "compass") {
        this->control->getActionDatabase()->fireActivateAction(Action::COMPASS);
        syncToolHighlight();
    } else if (actionId == "spacer") {
        this->control->selectTool(TOOL_VERTICAL_SPACE);
        syncToolHighlight();
    } else if (actionId == "laser") {
        this->control->selectTool(TOOL_LASER_POINTER_PEN);
        syncToolHighlight();
    } else if (actionId == "hand") {
        this->control->selectTool(TOOL_HAND);
        syncToolHighlight();
    } else if (actionId == "blackboard") {
        toggleBlackboardMode();
    } else if (actionId == "close_up") {
        toggleCloseUpPanel();
    } else if (actionId == "new_page") {
        this->control->getActionDatabase()->fireActivateAction(Action::NEW_PAGE_AFTER);
    } else if (actionId == "toggle_theme") {
        toggleLightDarkTheme();
    } else if (actionId == "focus_mode") {
        toggleFocusMode();
    } else if (actionId == "fullscreen") {
        this->fullscreenActive = !this->fullscreenActive;
        this->win->setFullscreen(this->fullscreenActive);
    }
}

void SwiftShell::rebuildPenSlotButtons() {
    if (!this->penSlotsBox) {
        return;
    }
    GList* children = gtk_container_get_children(GTK_CONTAINER(this->penSlotsBox));
    for (GList* it = children; it != nullptr; it = it->next) {
        gtk_widget_destroy(GTK_WIDGET(it->data));
    }
    g_list_free(children);
    this->penSlotButtons.clear();

    const ToolType curTool = this->control ? this->control->getToolHandler()->getToolType() : TOOL_PEN;
    const bool isWriting = (curTool == TOOL_PEN || curTool == TOOL_HIGHLIGHTER);

    for (uint32_t i = 0; i < static_cast<uint32_t>(this->penSlots.size()); ++i) {
        const auto& slot = this->penSlots[i];
        GtkWidget* btn = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
        gtk_widget_set_can_focus(btn, FALSE);
        setCssClass(btn, "swift-pen-slot", true);

        const std::string keyLabel = getShortcutKeyDisplay("pen_" + std::to_string(i + 1));
        char tip[180];
        if (!keyLabel.empty() && keyLabel != "—") {
            std::snprintf(tip, sizeof(tip), "%s (%.1f pt)  [%s]  •  bấm lại để chỉnh",
                          slot.label.empty() ? "Bút" : slot.label.c_str(), slot.widthPt, keyLabel.c_str());
        } else {
            std::snprintf(tip, sizeof(tip), "%s (%.1f pt)  •  bấm lại để chỉnh",
                          slot.label.empty() ? "Bút" : slot.label.c_str(), slot.widthPt);
        }
        gtk_widget_set_tooltip_text(btn, tip);

        GtkWidget* area = gtk_drawing_area_new();
        gtk_widget_set_size_request(area, 24, 24);
        auto* iconData = new PenIconData{uint32_t(slot.color) & 0x00FFFFFFU, slot.widthPt,
                                         slot.kind == SN_PEN_KIND_HIGHLIGHTER,
                                         isWriting && this->settings.activePenSlot == i, this->win};
        g_object_set_data(G_OBJECT(btn), "icon-data", iconData);
        g_signal_connect(area, "draw", G_CALLBACK(drawPenSlotIcon), iconData);
        g_signal_connect(area, "destroy",
                         G_CALLBACK(+[](GtkWidget*, gpointer ptr) { delete static_cast<PenIconData*>(ptr); }),
                         iconData);
        gtk_container_add(GTK_CONTAINER(btn), area);

        g_object_set_data(G_OBJECT(btn), "slot-index", GUINT_TO_POINTER(i));
        g_signal_connect(btn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "slot-index"));
                             const ToolType cur = self->control->getToolHandler()->getToolType();
                             const bool writing = (cur == TOOL_PEN || cur == TOOL_HIGHLIGHTER);
                             if (writing && self->settings.activePenSlot == idx) {
                                 self->openPenSlotPopover(idx, GTK_WIDGET(b));
                             } else {
                                 self->selectPenSlot(idx);
                             }
                         }),
                         this);
        g_signal_connect(btn, "button-press-event",
                         G_CALLBACK(+[](GtkWidget* w, GdkEventButton* ev, gpointer selfPtr) -> gboolean {
                             if (ev->button == 3) {
                                 auto* self = static_cast<SwiftShell*>(selfPtr);
                                 const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(w), "slot-index"));
                                 self->selectPenSlot(idx);
                                 self->openPenSlotPopover(idx, w);
                                 return TRUE;
                             }
                             return FALSE;
                         }),
                         this);

        gtk_box_pack_start(GTK_BOX(this->penSlotsBox), btn, FALSE, FALSE, 0);
        this->penSlotButtons.push_back(btn);
    }

    gtk_widget_show_all(this->penSlotsBox);
    syncToolHighlight();
}

void SwiftShell::applySlotToToolHandler(const PenSlot& slot) {
    ToolHandler* th = this->control->getToolHandler();
    const ToolType toolType = (slot.kind == SN_PEN_KIND_HIGHLIGHTER) ? TOOL_HIGHLIGHTER : TOOL_PEN;
    this->control->selectTool(toolType);
    th->setColor(slot.color, /*userSelection=*/false);
    th->setCustomThickness(toolType, slot.widthPt);
    th->setLineStyle(toXojLineStyle(slot.lineStyle));
    th->setDrawingType(slot.inkToShape ? DRAWING_TYPE_SHAPE_RECOGNIZER : DRAWING_TYPE_DEFAULT);
    this->control->getSettings()->setPressureSensitivity(slot.pressureSensitive);
}

void SwiftShell::selectPenSlot(uint32_t index) {
    if (index >= this->penSlots.size()) {
        return;
    }
    this->settings.activePenSlot = index;
    this->lastWritingSlot = index;
    applySlotToToolHandler(this->penSlots[index]);
    SwiftBridge::setSettings(this->settings);
    SwiftBridge::saveState();
    syncToolHighlight();
}

void SwiftShell::openPenSlotPopover(uint32_t index, GtkWidget* anchor) {
    if (index >= this->penSlots.size()) {
        return;
    }
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    gtk_widget_set_size_request(root, 264, -1);

    const PenSlot slot = this->penSlots[index];

    GtkWidget* previewArea = gtk_drawing_area_new();
    gtk_widget_set_size_request(previewArea, 244, 42);
    auto* previewData = new StrokePreviewData{uint32_t(slot.color) & 0x00FFFFFFU, slot.widthPt,
                                              slot.kind == SN_PEN_KIND_HIGHLIGHTER, slot.lineStyle};
    g_signal_connect(previewArea, "draw", G_CALLBACK(drawStrokePreview), previewData);
    g_signal_connect(previewArea, "destroy",
                     G_CALLBACK(+[](GtkWidget*, gpointer ptr) { delete static_cast<StrokePreviewData*>(ptr); }),
                     previewData);
    gtk_box_pack_start(GTK_BOX(root), previewArea, FALSE, FALSE, 0);

    GtkWidget* kindBox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    setCssClass(kindBox, "swift-segmented", true);
    GtkWidget* btnPen = gtk_button_new_with_label("Bút mực");
    GtkWidget* btnHi = gtk_button_new_with_label("Dạ quang");
    setCssClass(btnPen, "swift-active", slot.kind == SN_PEN_KIND_PEN);
    setCssClass(btnHi, "swift-active", slot.kind == SN_PEN_KIND_HIGHLIGHTER);
    gtk_box_pack_start(GTK_BOX(kindBox), btnPen, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(kindBox), btnHi, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(root), kindBox, FALSE, FALSE, 0);

    auto bindKind = [this, index](GtkWidget* btn, SnPenKind kind) {
        g_object_set_data(G_OBJECT(btn), "slot-index", GUINT_TO_POINTER(index));
        g_object_set_data(G_OBJECT(btn), "pen-kind", GINT_TO_POINTER(static_cast<int>(kind)));
        g_signal_connect(btn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "slot-index"));
                             const auto k = static_cast<SnPenKind>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "pen-kind")));
                             if (idx < self->penSlots.size()) {
                                 self->penSlots[idx].kind = k;
                                 if (k == SN_PEN_KIND_HIGHLIGHTER && self->penSlots[idx].widthPt < 6.0) {
                                     self->penSlots[idx].widthPt = 12.0;
                                 } else if (k == SN_PEN_KIND_PEN && self->penSlots[idx].widthPt > 6.0) {
                                     self->penSlots[idx].widthPt = 1.41;
                                 }
                                 SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                                 SwiftBridge::saveState();
                                 self->applySlotToToolHandler(self->penSlots[idx]);
                                 self->closeActivePopover();
                                 Util::execInUiThread([self, idx]() {
                                     self->rebuildPenSlotButtons();
                                     if (idx < self->penSlotButtons.size()) {
                                         self->openPenSlotPopover(idx, self->penSlotButtons[idx]);
                                     }
                                 });
                             }
                         }),
                         this);
    };
    bindKind(btnPen, SN_PEN_KIND_PEN);
    bindKind(btnHi, SN_PEN_KIND_HIGHLIGHTER);

    GtkWidget* styleBox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    setCssClass(styleBox, "swift-segmented", true);
    const std::array<std::pair<const char*, SnLineStyle>, 3> styles = {{
            {"Nét liền", SN_LINE_STYLE_SOLID},
            {"Nét đứt", SN_LINE_STYLE_DASHED},
            {"Chấm bi", SN_LINE_STYLE_DOTTED},
    }};
    for (const auto& [lbl, ls]: styles) {
        GtkWidget* b = gtk_button_new_with_label(lbl);
        setCssClass(b, "swift-active", slot.lineStyle == ls);
        g_object_set_data(G_OBJECT(b), "slot-index", GUINT_TO_POINTER(index));
        g_object_set_data(G_OBJECT(b), "line-style", GINT_TO_POINTER(static_cast<int>(ls)));
        g_signal_connect(b, "clicked", G_CALLBACK(+[](GtkButton* btn, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(btn), "slot-index"));
                             const auto st = static_cast<SnLineStyle>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(btn), "line-style")));
                             if (idx < self->penSlots.size()) {
                                 self->penSlots[idx].lineStyle = st;
                                 SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                                 SwiftBridge::saveState();
                                 self->applySlotToToolHandler(self->penSlots[idx]);
                                 self->closeActivePopover();
                                 Util::execInUiThread([self, idx]() {
                                     if (idx < self->penSlotButtons.size()) {
                                         self->openPenSlotPopover(idx, self->penSlotButtons[idx]);
                                     }
                                 });
                             }
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(styleBox), b, TRUE, TRUE, 0);
    }
    gtk_box_pack_start(GTK_BOX(root), styleBox, FALSE, FALSE, 0);

    GtkWidget* widthHeader = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    GtkWidget* widthTitle = gtk_label_new("Độ dày nét");
    setCssClass(widthTitle, "swift-section-title", true);
    gtk_label_set_xalign(GTK_LABEL(widthTitle), 0.0f);
    char widthBuf[32];
    std::snprintf(widthBuf, sizeof(widthBuf), "%.1f pt", slot.widthPt);
    GtkWidget* widthValLabel = gtk_label_new(widthBuf);
    setCssClass(widthValLabel, "swift-hint", true);
    gtk_box_pack_start(GTK_BOX(widthHeader), widthTitle, TRUE, TRUE, 0);
    gtk_box_pack_end(GTK_BOX(widthHeader), widthValLabel, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(root), widthHeader, FALSE, FALSE, 0);

    const double maxWidth = (slot.kind == SN_PEN_KIND_HIGHLIGHTER) ? 32.0 : 10.0;
    GtkWidget* scale = gtk_scale_new_with_range(GTK_ORIENTATION_HORIZONTAL, 0.4, maxWidth, 0.1);
    gtk_scale_set_draw_value(GTK_SCALE(scale), FALSE);
    gtk_range_set_value(GTK_RANGE(scale), std::clamp(slot.widthPt, 0.4, maxWidth));
    g_object_set_data(G_OBJECT(scale), "slot-index", GUINT_TO_POINTER(index));
    g_object_set_data(G_OBJECT(scale), "preview-area", previewArea);
    g_object_set_data(G_OBJECT(scale), "preview-data", previewData);
    g_object_set_data(G_OBJECT(scale), "val-label", widthValLabel);
    g_signal_connect(scale, "value-changed", G_CALLBACK(+[](GtkRange* range, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(range), "slot-index"));
                         const double v = gtk_range_get_value(range);
                         if (idx < self->penSlots.size()) {
                             self->penSlots[idx].widthPt = v;
                             SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                             SwiftBridge::saveState();
                             self->applySlotToToolHandler(self->penSlots[idx]);
                             if (auto* pd = static_cast<StrokePreviewData*>(
                                         g_object_get_data(G_OBJECT(range), "preview-data"))) {
                                 pd->widthPt = v;
                             }
                             if (auto* pa = GTK_WIDGET(g_object_get_data(G_OBJECT(range), "preview-area"))) {
                                 gtk_widget_queue_draw(pa);
                             }
                             if (auto* lbl = GTK_LABEL(g_object_get_data(G_OBJECT(range), "val-label"))) {
                                 char b[32];
                                 std::snprintf(b, sizeof(b), "%.1f pt", v);
                                 gtk_label_set_text(lbl, b);
                             }
                         }
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), scale, FALSE, FALSE, 0);

    GtkWidget* grid = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(grid), 6);
    gtk_grid_set_column_spacing(GTK_GRID(grid), 6);
    for (size_t cIdx = 0; cIdx < CURATED_INK_PALETTE.size(); ++cIdx) {
        const uint32_t rgb = CURATED_INK_PALETTE[cIdx];
        GtkWidget* sw = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(sw), GTK_RELIEF_NONE);
        setCssClass(sw, "swift-swatch", true);
        if ((uint32_t(slot.color) & 0x00FFFFFFU) == rgb) {
            setCssClass(sw, "swift-active", true);
        }

        GtkCssProvider* prov = gtk_css_provider_new();
        std::string css = "button { background-color: " + formatRgbCss(Color{rgb}) + "; }";
        gtk_css_provider_load_from_data(prov, css.c_str(), -1, nullptr);
        gtk_style_context_add_provider(gtk_widget_get_style_context(sw), GTK_STYLE_PROVIDER(prov),
                                       GTK_STYLE_PROVIDER_PRIORITY_USER);
        g_object_unref(prov);

        g_object_set_data(G_OBJECT(sw), "slot-index", GUINT_TO_POINTER(index));
        g_object_set_data(G_OBJECT(sw), "rgb", GUINT_TO_POINTER(rgb));
        g_signal_connect(sw, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "slot-index"));
                             const uint32_t rgbVal = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "rgb"));
                             if (idx < self->penSlots.size()) {
                                 self->penSlots[idx].color = Color{0xFF000000U | (rgbVal & 0x00FFFFFFU)};
                                 SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                                 SwiftBridge::saveState();
                                 self->applySlotToToolHandler(self->penSlots[idx]);
                                 self->closeActivePopover();
                                 Util::execInUiThread([self, idx]() {
                                     self->rebuildPenSlotButtons();
                                     if (idx < self->penSlotButtons.size()) {
                                         self->openPenSlotPopover(idx, self->penSlotButtons[idx]);
                                     }
                                 });
                             }
                         }),
                         this);
        gtk_grid_attach(GTK_GRID(grid), sw, static_cast<gint>(cIdx % 8), static_cast<gint>(cIdx / 8), 1, 1);
    }
    gtk_box_pack_start(GTK_BOX(root), grid, FALSE, FALSE, 0);

    GtkWidget* customColorBtn = makeMenuRow("preferences-color-symbolic", "Chọn màu tùy chỉnh...");
    g_object_set_data(G_OBJECT(customColorBtn), "slot-index", GUINT_TO_POINTER(index));
    g_signal_connect(customColorBtn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "slot-index"));
                         self->closeActivePopover();
                         if (idx >= self->penSlots.size()) {
                             return;
                         }
                         GtkWidget* dlg = gtk_color_chooser_dialog_new("Chọn màu mực", self->control->getGtkWindow());
                         GdkRGBA initRgba{};
                         colorToGdkRgba(self->penSlots[idx].color, initRgba);
                         gtk_color_chooser_set_rgba(GTK_COLOR_CHOOSER(dlg), &initRgba);
                         if (gtk_dialog_run(GTK_DIALOG(dlg)) == GTK_RESPONSE_OK) {
                             GdkRGBA chosen{};
                             gtk_color_chooser_get_rgba(GTK_COLOR_CHOOSER(dlg), &chosen);
                             self->penSlots[idx].color = gdkRgbaToColor(chosen);
                             SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                             SwiftBridge::saveState();
                             self->applySlotToToolHandler(self->penSlots[idx]);
                             self->rebuildPenSlotButtons();
                         }
                         gtk_widget_destroy(dlg);
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), customColorBtn, FALSE, FALSE, 0);

    auto addSwitchRow = [&](const char* labelText, bool initialVal, GCallback cb) {
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        GtkWidget* lbl = gtk_label_new(labelText);
        gtk_label_set_xalign(GTK_LABEL(lbl), 0.0f);
        GtkWidget* sw = gtk_switch_new();
        gtk_switch_set_active(GTK_SWITCH(sw), initialVal);
        g_object_set_data(G_OBJECT(sw), "slot-index", GUINT_TO_POINTER(index));
        g_signal_connect(sw, "state-set", cb, this);
        gtk_box_pack_start(GTK_BOX(row), lbl, TRUE, TRUE, 0);
        gtk_box_pack_end(GTK_BOX(row), sw, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(root), row, FALSE, FALSE, 0);
    };

    addSwitchRow("Nhạy áp lực bút", slot.pressureSensitive,
                 G_CALLBACK(+[](GtkSwitch* sw, gboolean state, gpointer selfPtr) -> gboolean {
                     auto* self = static_cast<SwiftShell*>(selfPtr);
                     const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(sw), "slot-index"));
                     if (idx < self->penSlots.size()) {
                         self->penSlots[idx].pressureSensitive = (state != FALSE);
                         SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                         SwiftBridge::saveState();
                         self->applySlotToToolHandler(self->penSlots[idx]);
                     }
                     return FALSE;
                 }));

    addSwitchRow("Tự nắn hình (Ink-to-Shape)", slot.inkToShape,
                 G_CALLBACK(+[](GtkSwitch* sw, gboolean state, gpointer selfPtr) -> gboolean {
                     auto* self = static_cast<SwiftShell*>(selfPtr);
                     const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(sw), "slot-index"));
                     if (idx < self->penSlots.size()) {
                         self->penSlots[idx].inkToShape = (state != FALSE);
                         SwiftBridge::setPenSlot(idx, self->penSlots[idx]);
                         SwiftBridge::saveState();
                         self->applySlotToToolHandler(self->penSlots[idx]);
                     }
                     return FALSE;
                 }));

    GtkWidget* footer = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    if (this->penSlots.size() < 8) {
        GtkWidget* addBtn = makeMenuRow("list-add-symbolic", "Thêm slot");
        g_signal_connect(addBtn, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             PenSlot newSlot;
                             newSlot.label = "Bút mới";
                             newSlot.kind = SN_PEN_KIND_PEN;
                             newSlot.color = Color{0xFF4F46E5U};
                             newSlot.widthPt = 1.6;
                             if (auto newIdx = SwiftBridge::addPenSlot(newSlot)) {
                                 SwiftBridge::saveState();
                                 self->penSlots = SwiftBridge::getPenSlots();
                                 self->closeActivePopover();
                                 const uint32_t targetIdx = *newIdx;
                                 Util::execInUiThread([self, targetIdx]() {
                                     self->rebuildPenSlotButtons();
                                     self->selectPenSlot(targetIdx);
                                 });
                             }
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(footer), addBtn, TRUE, TRUE, 0);
    }
    if (this->penSlots.size() > 1) {
        GtkWidget* delBtn = makeMenuRow("user-trash-symbolic", "Xóa slot");
        g_object_set_data(G_OBJECT(delBtn), "slot-index", GUINT_TO_POINTER(index));
        g_signal_connect(delBtn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const uint32_t idx = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "slot-index"));
                             if (SwiftBridge::removePenSlot(idx)) {
                                 SwiftBridge::saveState();
                                 self->penSlots = SwiftBridge::getPenSlots();
                                 self->settings = SwiftBridge::getSettings();
                                 self->closeActivePopover();
                                 Util::execInUiThread([self]() {
                                     self->rebuildPenSlotButtons();
                                     self->selectPenSlot(self->settings.activePenSlot);
                                 });
                             }
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(footer), delBtn, TRUE, TRUE, 0);
    }
    gtk_box_pack_start(GTK_BOX(root), footer, FALSE, FALSE, 0);

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openEraserPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_widget_set_size_request(root, 236, -1);

    ToolHandler* th = this->control->getToolHandler();
    const EraserType curType = th->getEraserType();

    const std::array<std::pair<const char*, EraserType>, 3> modes = {{
            {"Tẩy cả nét (Delete Stroke)", ERASER_TYPE_DELETE_STROKE},
            {"Tẩy từng phần (Standard)", ERASER_TYPE_DEFAULT},
            {"Tẩy đè trắng (Whiteout)", ERASER_TYPE_WHITEOUT},
    }};
    for (const auto& [label, et]: modes) {
        GtkWidget* row = makeMenuRow("xopp-tool-eraser", label, curType == et);
        g_object_set_data(G_OBJECT(row), "eraser-type", GINT_TO_POINTER(static_cast<int>(et)));
        g_signal_connect(row, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const auto mode = static_cast<EraserType>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "eraser-type")));
                             self->control->selectTool(TOOL_ERASER);
                             self->control->getToolHandler()->setEraserType(mode);
                             self->closeActivePopover();
                             self->syncToolHighlight();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(root), row, FALSE, FALSE, 0);
    }

    GtkWidget* sizeBox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    setCssClass(sizeBox, "swift-segmented", true);
    const std::array<std::pair<const char*, ToolSize>, 4> sizes = {{
            {"Nhỏ", TOOL_SIZE_FINE},
            {"Vừa", TOOL_SIZE_MEDIUM},
            {"Lớn", TOOL_SIZE_THICK},
            {"Rất lớn", TOOL_SIZE_VERY_THICK},
    }};
    const ToolSize curSize = th->getSize(SelectedTool::active);
    for (const auto& [lbl, sz]: sizes) {
        GtkWidget* b = gtk_button_new_with_label(lbl);
        setCssClass(b, "swift-active", curSize == sz);
        g_object_set_data(G_OBJECT(b), "eraser-size", GINT_TO_POINTER(static_cast<int>(sz)));
        g_signal_connect(b, "clicked", G_CALLBACK(+[](GtkButton* btn, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const auto s = static_cast<ToolSize>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(btn), "eraser-size")));
                             self->control->selectTool(TOOL_ERASER);
                             self->control->getToolHandler()->setEraserSize(s);
                             self->closeActivePopover();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(sizeBox), b, TRUE, TRUE, 0);
    }
    gtk_box_pack_start(GTK_BOX(root), sizeBox, FALSE, FALSE, 2);

    GtkWidget* retRow = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    GtkWidget* retLbl = gtk_label_new("Tự quay lại bút sau khi tẩy");
    gtk_label_set_xalign(GTK_LABEL(retLbl), 0.0f);
    GtkWidget* retSw = gtk_switch_new();
    gtk_switch_set_active(GTK_SWITCH(retSw), this->settings.eraserAutoReturn);
    g_signal_connect(retSw, "state-set", G_CALLBACK(+[](GtkSwitch*, gboolean state, gpointer selfPtr) -> gboolean {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         self->settings.eraserAutoReturn = (state != FALSE);
                         SwiftBridge::setSettings(self->settings);
                         SwiftBridge::saveState();
                         return FALSE;
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(retRow), retLbl, TRUE, TRUE, 0);
    gtk_box_pack_end(GTK_BOX(retRow), retSw, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(root), retRow, FALSE, FALSE, 2);

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openSelectionPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3);
    const ToolType cur = this->control->getToolHandler()->getToolType();

    const std::array<std::tuple<const char*, const char*, ToolType>, 4> items = {{
            {"xopp-select-lasso", "Chọn vùng tự do (Lasso)", TOOL_SELECT_REGION},
            {"xopp-select-rect", "Chọn hình chữ nhật (Box)", TOOL_SELECT_RECT},
            {"xopp-object-select", "Chọn đối tượng", TOOL_SELECT_OBJECT},
            {"xopp-select-pdf-text-area", "Chọn văn bản PDF", TOOL_SELECT_PDF_TEXT_LINEAR},
    }};
    for (const auto& [icon, label, tt]: items) {
        GtkWidget* row = makeMenuRow(icon, label, cur == tt);
        g_object_set_data(G_OBJECT(row), "tool-type", GINT_TO_POINTER(static_cast<int>(tt)));
        g_signal_connect(row, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const auto t = static_cast<ToolType>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "tool-type")));
                             self->control->selectTool(t);
                             self->closeActivePopover();
                             self->syncToolHighlight();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(root), row, FALSE, FALSE, 0);
    }

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openShapesPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3);
    const DrawingType cur = this->control->getToolHandler()->getDrawingType();

    const std::array<std::tuple<const char*, const char*, DrawingType>, 8> items = {{
            {"xopp-tool-pencil", "Vẽ tự do", DRAWING_TYPE_DEFAULT},
            {"xopp-shape-recognizer", "Ink-to-Shape (Tự nắn hình)", DRAWING_TYPE_SHAPE_RECOGNIZER},
            {"xopp-draw-line", "Đường thẳng", DRAWING_TYPE_LINE},
            {"xopp-draw-arrow", "Mũi tên", DRAWING_TYPE_ARROW},
            {"xopp-draw-rect", "Hình chữ nhật", DRAWING_TYPE_RECTANGLE},
            {"xopp-draw-ellipse", "Hình tròn / Elip", DRAWING_TYPE_ELLIPSE},
            {"xopp-draw-coordinate-system", "Hệ trục tọa độ Oxy", DRAWING_TYPE_COORDINATE_SYSTEM},
            {"xopp-draw-spline", "Đường cong Spline", DRAWING_TYPE_SPLINE},
    }};
    for (const auto& [icon, label, dt]: items) {
        GtkWidget* row = makeMenuRow(icon, label, cur == dt);
        g_object_set_data(G_OBJECT(row), "draw-type", GINT_TO_POINTER(static_cast<int>(dt)));
        g_signal_connect(row, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const auto d = static_cast<DrawingType>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "draw-type")));
                             auto* th = self->control->getToolHandler();
                             if (th->getToolType() != TOOL_PEN && th->getToolType() != TOOL_HIGHLIGHTER) {
                                 self->control->selectTool(TOOL_PEN);
                             }
                             th->setDrawingType(d);
                             self->closeActivePopover();
                             self->syncToolHighlight();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(root), row, FALSE, FALSE, 0);
    }

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openTextPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    gtk_widget_set_size_request(root, 248, -1);

    Settings* st = this->control->getSettings();
    XojFont curFont = st->getFont();

    GtkWidget* sizeHdr = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    GtkWidget* sizeTitle = gtk_label_new("Cỡ chữ văn bản");
    setCssClass(sizeTitle, "swift-section-title", true);
    gtk_label_set_xalign(GTK_LABEL(sizeTitle), 0.0f);
    char szBuf[24];
    std::snprintf(szBuf, sizeof(szBuf), "%.0f pt", curFont.getSize());
    GtkWidget* sizeValLbl = gtk_label_new(szBuf);
    setCssClass(sizeValLbl, "swift-hint", true);
    gtk_box_pack_start(GTK_BOX(sizeHdr), sizeTitle, TRUE, TRUE, 0);
    gtk_box_pack_end(GTK_BOX(sizeHdr), sizeValLbl, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(root), sizeHdr, FALSE, FALSE, 0);

    GtkWidget* sizeScale = gtk_scale_new_with_range(GTK_ORIENTATION_HORIZONTAL, 8.0, 48.0, 1.0);
    gtk_scale_set_draw_value(GTK_SCALE(sizeScale), FALSE);
    gtk_range_set_value(GTK_RANGE(sizeScale), std::clamp(curFont.getSize(), 8.0, 48.0));
    g_object_set_data(G_OBJECT(sizeScale), "val-label", sizeValLbl);
    g_signal_connect(sizeScale, "value-changed", G_CALLBACK(+[](GtkRange* r, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         const double sz = gtk_range_get_value(r);
                         XojFont f = self->control->getSettings()->getFont();
                         f.setSize(sz);
                         self->control->getSettings()->setFont(f);
                         if (auto* te = self->win && self->win->getXournal() ? self->win->getXournal()->getTextEditor() : nullptr) {
                             te->setFont(f);
                         }
                         if (auto* lbl = GTK_LABEL(g_object_get_data(G_OBJECT(r), "val-label"))) {
                             char b[24];
                             std::snprintf(b, sizeof(b), "%.0f pt", sz);
                             gtk_label_set_text(lbl, b);
                         }
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), sizeScale, FALSE, FALSE, 0);

    GtkWidget* colTitle = gtk_label_new("Màu chữ");
    setCssClass(colTitle, "swift-section-title", true);
    gtk_label_set_xalign(GTK_LABEL(colTitle), 0.0f);
    gtk_box_pack_start(GTK_BOX(root), colTitle, FALSE, FALSE, 0);

    const Color curCol = this->control->getToolHandler()->getTool(TOOL_TEXT).getColor();
    GtkWidget* grid = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(grid), 6);
    gtk_grid_set_column_spacing(GTK_GRID(grid), 6);
    for (size_t cIdx = 0; cIdx < CURATED_INK_PALETTE.size(); ++cIdx) {
        const uint32_t rgb = CURATED_INK_PALETTE[cIdx];
        GtkWidget* sw = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(sw), GTK_RELIEF_NONE);
        setCssClass(sw, "swift-swatch", true);
        if ((uint32_t(curCol) & 0x00FFFFFFU) == rgb) {
            setCssClass(sw, "swift-active", true);
        }
        GtkCssProvider* prov = gtk_css_provider_new();
        std::string css = "button { background-color: " + formatRgbCss(Color{rgb}) + "; }";
        gtk_css_provider_load_from_data(prov, css.c_str(), -1, nullptr);
        gtk_style_context_add_provider(gtk_widget_get_style_context(sw), GTK_STYLE_PROVIDER(prov),
                                       GTK_STYLE_PROVIDER_PRIORITY_USER);
        g_object_unref(prov);
        g_object_set_data(G_OBJECT(sw), "rgb", GUINT_TO_POINTER(rgb));
        g_signal_connect(sw, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const uint32_t rgbVal = GPOINTER_TO_UINT(g_object_get_data(G_OBJECT(b), "rgb"));
                             const Color c{0xFF000000U | (rgbVal & 0x00FFFFFFU)};
                             self->control->selectTool(TOOL_TEXT);
                             self->control->getToolHandler()->setColor(c, /*userSelection=*/true);
                             if (auto* te = self->win && self->win->getXournal() ? self->win->getXournal()->getTextEditor() : nullptr) {
                                 te->setColor(c);
                             }
                             self->closeActivePopover();
                         }),
                         this);
        gtk_grid_attach(GTK_GRID(grid), sw, static_cast<gint>(cIdx % 8), static_cast<gint>(cIdx / 8), 1, 1);
    }
    gtk_box_pack_start(GTK_BOX(root), grid, FALSE, FALSE, 0);

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openRulerPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3);

    const bool hasSq = this->control->hasSetsquare();
    const bool hasCp = this->control->hasCompass();

    GtkWidget* btnSetsquare = makeMenuRow("xopp-setsquare", hasSq ? "Ẩn thước ê-ke & Đo góc" : "Thước ê-ke & Đo góc", hasSq);
    g_signal_connect(btnSetsquare, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         self->closeActivePopover();
                         Util::execInUiThread([self]() {
                             self->control->getActionDatabase()->fireActivateAction(Action::SETSQUARE);
                             self->syncToolHighlight();
                             if (self->win && self->win->getXournal()) {
                                 self->win->getXournal()->requestFocus();
                                 gtk_widget_queue_draw(self->win->getXournal()->getWidget());
                             }
                         });
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), btnSetsquare, FALSE, FALSE, 0);

    GtkWidget* btnCompass = makeMenuRow("xopp-compass", hasCp ? "Ẩn Compa" : "Compa vẽ cung & đo góc", hasCp);
    g_signal_connect(btnCompass, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         self->closeActivePopover();
                         Util::execInUiThread([self]() {
                             self->control->getActionDatabase()->fireActivateAction(Action::COMPASS);
                             self->syncToolHighlight();
                             if (self->win && self->win->getXournal()) {
                                 self->win->getXournal()->requestFocus();
                                 gtk_widget_queue_draw(self->win->getXournal()->getWidget());
                             }
                         });
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), btnCompass, FALSE, FALSE, 0);

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openLaserHandPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3);
    const ToolType cur = this->control->getToolHandler()->getToolType();

    GtkWidget* btnLaser = makeMenuRow("xopp-laser-pointer", "Con trỏ Laser", cur == TOOL_LASER_POINTER_PEN);
    g_signal_connect(btnLaser, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         self->control->selectTool(TOOL_LASER_POINTER_PEN);
                         self->closeActivePopover();
                         self->syncToolHighlight();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), btnLaser, FALSE, FALSE, 0);

    GtkWidget* btnHand = makeMenuRow("xopp-hand", "Chế độ cuộn tay (Hand)", cur == TOOL_HAND);
    g_signal_connect(btnHand, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         self->control->selectTool(TOOL_HAND);
                         self->closeActivePopover();
                         self->syncToolHighlight();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), btnHand, FALSE, FALSE, 0);

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openPaperPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3);
    gtk_widget_set_size_request(root, 224, -1);

    const std::array<std::pair<const char*, const char*>, 9> presets = {{
            {"Giấy trắng trơn (Plain)", "plain"},
            {"Kẻ ngang (Ruled)", "ruled"},
            {"Kẻ ngang có lề (Lined)", "lined"},
            {"Cornell Notes", "cornell"},
            {"Ô ly vuông (Graph)", "graph"},
            {"Chấm bi (Dotted)", "dotted"},
            {"Lưới Isometric", "isograph"},
            {"Khuông nhạc (Staves)", "staves"},
            {"Bảng đen (Blackboard)", "blackboard"},
    }};
    for (const auto& [label, id]: presets) {
        GtkWidget* row = makeMenuRow("xopp-sidebar-page-preview", label);
        g_object_set_data_full(G_OBJECT(row), "tpl-id", g_strdup(id), g_free);
        g_signal_connect(row, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const char* tpl = static_cast<const char*>(g_object_get_data(G_OBJECT(b), "tpl-id"));
                             if (tpl) {
                                 self->applyPaperTemplate(tpl, /*allPages=*/false);
                             }
                             self->closeActivePopover();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(root), row, FALSE, FALSE, 0);
    }

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::openFileMenuPopover(GtkWidget* anchor) {
    GtkWidget* pop = makePopover(anchor);
    GtkWidget* root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_widget_set_size_request(root, 240, -1);

    auto addRow = [&](const char* icon, const char* label, GCallback cb) {
        GtkWidget* row = makeMenuRow(icon, label);
        g_signal_connect(row, "clicked", cb, this);
        gtk_box_pack_start(GTK_BOX(root), row, FALSE, FALSE, 0);
    };

    addRow("xopp-document-new", "Ghi chú mới", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->control->newFile();
           }));
    addRow("xopp-document-open", "Mở tệp / PDF (Ctrl+O)", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->control->askToOpenFile();
           }));
    addRow("xopp-document-save", "Lưu (Ctrl+S)", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->control->save();
           }));
    addRow("xopp-document-save", "Lưu thành...", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->control->saveAs();
           }));

    gtk_box_pack_start(GTK_BOX(root), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL), FALSE, FALSE, 3);

    if (DisplayState::isBlackboardActive()) {
        addRow("xopp-document-export-pdf", "Xuất PDF (Giữ Bảng đen)", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                   auto* self = static_cast<SwiftShell*>(selfPtr);
                   self->closeActivePopover();
                   self->control->exportAsPdf(/*forceLightPaper=*/false);
               }));
        addRow("xopp-document-export-pdf", "Xuất PDF (Nền giấy để in)",
               G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                   auto* self = static_cast<SwiftShell*>(selfPtr);
                   self->closeActivePopover();
                   self->control->exportAsPdf(/*forceLightPaper=*/true);
               }));
    } else {
        addRow("xopp-document-export-pdf", "Xuất PDF", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                   auto* self = static_cast<SwiftShell*>(selfPtr);
                   self->closeActivePopover();
                   self->control->exportAsPdf(false);
               }));
    }
    addRow("xopp-document-export-pdf", "Xuất PNG / SVG...", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->control->exportAs();
           }));

    gtk_box_pack_start(GTK_BOX(root), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL), FALSE, FALSE, 3);

    GtkWidget* paperBtn = makeMenuRow("xopp-sidebar-page-preview", "Đổi mẫu giấy...");
    g_object_set_data(G_OBJECT(paperBtn), "anchor", anchor);
    g_signal_connect(paperBtn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         GtkWidget* a = GTK_WIDGET(g_object_get_data(G_OBJECT(b), "anchor"));
                         self->closeActivePopover();
                         Util::execInUiThread([self, a]() { self->openPaperPopover(a); });
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(root), paperBtn, FALSE, FALSE, 0);

    addRow("xopp-tool-image", "Dán ảnh Clipboard (Ctrl+V)", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->control->getActionDatabase()->fireActivateAction(Action::PASTE);
           }));

    gtk_box_pack_start(GTK_BOX(root), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL), FALSE, FALSE, 3);

    addRow("emblem-system-symbolic", "Cài đặt Swift note...", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
               auto* self = static_cast<SwiftShell*>(selfPtr);
               self->closeActivePopover();
               self->openSettingsDialog();
           }));

    gtk_container_add(GTK_CONTAINER(pop), root);
    gtk_widget_show_all(root);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftShell::applyPaperTemplate(const std::string& templateId, bool allPages) {
    PageType pt;
    if (templateId == "plain") {
        pt.format = PageTypeFormat::Plain;
    } else if (templateId == "ruled") {
        pt.format = PageTypeFormat::Ruled;
    } else if (templateId == "lined") {
        pt.format = PageTypeFormat::Lined;
    } else if (templateId == "cornell") {
        pt.format = PageTypeFormat::Lined;
        pt.config = "m1=170,cornell=1";
    } else if (templateId == "graph") {
        pt.format = PageTypeFormat::Graph;
    } else if (templateId == "dotted") {
        pt.format = PageTypeFormat::Dotted;
    } else if (templateId == "isograph") {
        pt.format = PageTypeFormat::IsoGraph;
    } else if (templateId == "staves") {
        pt.format = PageTypeFormat::Staves;
    } else if (templateId == "blackboard") {
        if (!this->settings.blackboardEnabled) {
            toggleBlackboardMode();
        }
        return;
    } else {
        pt.format = PageTypeFormat::Ruled;
    }

    auto* bgCtrl = this->control->getPageBackgroundChangeController();
    if (allPages) {
        bgCtrl->applyBackgroundToAllPages(pt);
    } else {
        bgCtrl->changeCurrentPageBackground(pt);
    }
    bgCtrl->setPageTypeForNewPages(pt);
}

void SwiftShell::toggleBlackboardMode() {
    this->settings.blackboardEnabled = !this->settings.blackboardEnabled;
    SwiftBridge::setSettings(this->settings);
    SwiftBridge::saveState();
    refreshCanvasForDisplayState();
    rebuildPenSlotButtons();
    syncToolHighlight();
}

void SwiftShell::toggleLightDarkTheme() {
    this->settings.theme = (this->settings.theme == SN_THEME_DARK) ? SN_THEME_LIGHT : SN_THEME_DARK;
    SwiftBridge::setSettings(this->settings);
    SwiftBridge::saveState();
    applyThemeFromSettings();
}

void SwiftShell::applyThemeFromSettings() {
    if (!control || !control->getSettings() || !win) {
        return;
    }
    ThemeVariant variant = THEME_VARIANT_FORCE_LIGHT;
    if (this->settings.theme == SN_THEME_DARK) {
        variant = THEME_VARIANT_FORCE_DARK;
    } else if (this->settings.theme == SN_THEME_SYSTEM) {
        variant = THEME_VARIANT_USE_SYSTEM;
    }
    control->getSettings()->setThemeVariant(variant);
    win->updateColorscheme();

    if (this->logoArea) {
        gtk_widget_queue_draw(this->logoArea);
    }
    for (GtkWidget* btn: this->penSlotButtons) {
        if (GtkWidget* area = gtk_bin_get_child(GTK_BIN(btn))) {
            gtk_widget_queue_draw(area);
        }
    }
}

void SwiftShell::refreshCanvasForDisplayState() {
    if (!win || !win->getXournal()) {
        return;
    }
    for (const auto& pv: win->getXournal()->getViewPages()) {
        if (pv) {
            pv->rerenderPage();
        }
    }
    if (this->closeUpPanel) {
        this->closeUpPanel->queueRedraw();
    }
}

void SwiftShell::onEraseStrokeFinished() {
    if (!this->settings.eraserAutoReturn || !this->control) {
        return;
    }
    ToolHandler* th = this->control->getToolHandler();
    if (!th || th->getToolType() != TOOL_ERASER) {
        return;
    }
    Util::execInUiThread([this]() {
        if (!this->settings.eraserAutoReturn || !this->control) {
            return;
        }
        if (this->control->getToolHandler()->getToolType() == TOOL_ERASER &&
            this->lastWritingSlot < this->penSlots.size()) {
            this->selectPenSlot(this->lastWritingSlot);
        }
    });
}

void SwiftShell::updateDocumentTitle(const std::string& title) {
    if (this->docTitleLabel) {
        gtk_label_set_text(GTK_LABEL(this->docTitleLabel), title.empty() ? "Ghi chú mới" : title.c_str());
    }
}

void SwiftShell::updatePageInfo(size_t currentPageZeroBased, size_t pageCount) {
    if (!this->pageInfoLabel) {
        return;
    }
    char buf[48];
    const size_t shown = (pageCount == 0) ? 0 : (currentPageZeroBased + 1);
    std::snprintf(buf, sizeof(buf), "%zu / %zu", shown, pageCount);
    gtk_label_set_text(GTK_LABEL(this->pageInfoLabel), buf);
}

void SwiftShell::syncToolHighlight() {
    if (!this->control) {
        return;
    }
    ToolHandler* th = this->control->getToolHandler();
    const ToolType cur = th->getToolType();
    const DrawingType dt = th->getDrawingType();

    const bool isWritingTool = (cur == TOOL_PEN || cur == TOOL_HIGHLIGHTER);
    for (uint32_t i = 0; i < static_cast<uint32_t>(this->penSlotButtons.size()); ++i) {
        const bool isActive = isWritingTool && this->settings.activePenSlot == i;
        setCssClass(this->penSlotButtons[i], "swift-active", isActive);
        if (auto* iconData = static_cast<PenIconData*>(g_object_get_data(G_OBJECT(this->penSlotButtons[i]), "icon-data"))) {
            iconData->active = isActive;
        }
        if (GtkWidget* area = gtk_bin_get_child(GTK_BIN(this->penSlotButtons[i]))) {
            gtk_widget_queue_draw(area);
        }
    }

    setCssClass(this->btnEraser, "swift-active", cur == TOOL_ERASER);
    setCssClass(this->btnSelection, "swift-active",
                cur == TOOL_SELECT_REGION || cur == TOOL_SELECT_RECT || cur == TOOL_SELECT_OBJECT ||
                        cur == TOOL_SELECT_PDF_TEXT_LINEAR || cur == TOOL_SELECT_PDF_TEXT_RECT);
    setCssClass(this->btnShapes, "swift-active", isWritingTool && dt != DRAWING_TYPE_DEFAULT);
    setCssClass(this->btnText, "swift-active", cur == TOOL_TEXT);
    setCssClass(this->btnRuler, "swift-active", this->control->hasSetsquare() || this->control->hasCompass());
    setCssClass(this->btnSpacer, "swift-active", cur == TOOL_VERTICAL_SPACE);
    setCssClass(this->btnLaserHand, "swift-active", cur == TOOL_LASER_POINTER_PEN || cur == TOOL_HAND);
    setCssClass(this->btnBlackboard, "swift-blackboard-on", this->settings.blackboardEnabled);
    setCssClass(this->btnCloseUp, "swift-active", this->closeUpPanel && this->closeUpPanel->isVisible());
    setCssClass(this->btnSidebar, "swift-active", this->control->getSettings()->isSidebarVisible());
}

void SwiftShell::showEditor() {
    if (this->stack) {
        gtk_stack_set_visible_child_name(GTK_STACK(this->stack), "editor");
    }
}

void SwiftShell::showLibrary() {
    if (this->libraryView && this->stack) {
        this->libraryView->refresh();
        gtk_stack_set_visible_child_name(GTK_STACK(this->stack), "library");
    } else if (this->control) {
        this->control->askToOpenFile();
    }
}

void SwiftShell::toggleCloseUpPanel() {
    if (this->closeUpPanel) {
        this->closeUpPanel->toggle();
    }
}

namespace {
struct ShortcutRecorderState {
    SwiftShell* shell = nullptr;
    GtkWidget* activeButton = nullptr;
    std::string activeActionId;
    std::vector<std::pair<std::string, GtkWidget*>> keyButtons;
};
}  // namespace

void SwiftShell::openSettingsDialog() {
    GtkWidget* dlg = gtk_dialog_new_with_buttons("Cài đặt Swift note", this->control->getGtkWindow(),
                                                 static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL |
                                                                             GTK_DIALOG_DESTROY_WITH_PARENT),
                                                 "Xong", GTK_RESPONSE_CLOSE, nullptr);
    gtk_window_set_default_size(GTK_WINDOW(dlg), 660, 490);
    setCssClass(dlg, "swift-dialog", true);
    if (win && win->isDarkTheme()) {
        setCssClass(dlg, "darkMode", true);
    }

    GtkWidget* contentArea = gtk_dialog_get_content_area(GTK_DIALOG(dlg));
    GtkWidget* split = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);

    // === LEFT SIDEBAR NAVIGATION (4 clean macOS-style tabs) ===
    GtkWidget* sidebar = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    gtk_widget_set_name(sidebar, "swiftSettingsSidebar");
    gtk_widget_set_size_request(sidebar, 178, -1);

    GtkWidget* tabStack = gtk_stack_new();
    gtk_widget_set_name(tabStack, "swiftSettingsContent");
    gtk_stack_set_transition_type(GTK_STACK(tabStack), GTK_STACK_TRANSITION_TYPE_CROSSFADE);
    gtk_stack_set_transition_duration(GTK_STACK(tabStack), 110);

    auto* tabButtons = new std::vector<GtkWidget*>();

    auto addNavTab = [&](const char* id, const char* iconName, const char* label, bool initial) {
        GtkWidget* b = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(b), GTK_RELIEF_NONE);
        setCssClass(b, "swift-tab-btn", true);
        if (initial) {
            setCssClass(b, "swift-active", true);
        }
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        GtkWidget* img = gtk_image_new_from_icon_name(iconName, GTK_ICON_SIZE_MENU);
        gtk_image_set_pixel_size(GTK_IMAGE(img), 15);
        GtkWidget* lbl = gtk_label_new(label);
        gtk_label_set_xalign(GTK_LABEL(lbl), 0.0f);
        gtk_box_pack_start(GTK_BOX(row), img, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(row), lbl, TRUE, TRUE, 0);
        gtk_container_add(GTK_CONTAINER(b), row);

        g_object_set_data_full(G_OBJECT(b), "tab-id", g_strdup(id), g_free);
        g_object_set_data(G_OBJECT(b), "tab-stack", tabStack);
        g_object_set_data(G_OBJECT(b), "tab-list", tabButtons);
        g_signal_connect(b, "clicked", G_CALLBACK(+[](GtkButton* btn, gpointer) {
                             const char* tid = static_cast<const char*>(g_object_get_data(G_OBJECT(btn), "tab-id"));
                             auto* st = GTK_STACK(g_object_get_data(G_OBJECT(btn), "tab-stack"));
                             auto* list = static_cast<std::vector<GtkWidget*>*>(
                                     g_object_get_data(G_OBJECT(btn), "tab-list"));
                             if (tid && st) {
                                 gtk_stack_set_visible_child_name(st, tid);
                             }
                             if (list) {
                                 for (GtkWidget* tb: *list) {
                                     setCssClass(tb, "swift-active", tb == GTK_WIDGET(btn));
                                 }
                             }
                         }),
                         nullptr);
        gtk_box_pack_start(GTK_BOX(sidebar), b, FALSE, FALSE, 0);
        tabButtons->push_back(b);
    };

    addNavTab("appearance", "preferences-desktop-theme-symbolic", "Giao diện", true);
    addNavTab("blackboard", "xopp-presentation-mode", "Bảng đen & Giấy", false);
    addNavTab("pen", "xopp-tool-pencil", "Bút & Phóng to", false);
    addNavTab("shortcuts", "input-keyboard-symbolic", "Phím tắt", false);

    // =========================================================================
    // TAB 1: GIAO DIỆN (3 Switchable Layouts + Light / Dark / System)
    // =========================================================================
    GtkWidget* tabAppearance = gtk_box_new(GTK_ORIENTATION_VERTICAL, 14);
    GtkWidget* h1 = gtk_label_new("Giao diện & Bố cục");
    setCssClass(h1, "swift-settings-heading", true);
    gtk_label_set_xalign(GTK_LABEL(h1), 0.0f);
    gtk_box_pack_start(GTK_BOX(tabAppearance), h1, FALSE, FALSE, 0);

    GtkWidget* layoutDesc = gtk_label_new("Chuyển đổi tức thì giữa 3 bố cục tối giản (nhấn Tab để ẩn/hiện thanh công cụ khi viết).");
    setCssClass(layoutDesc, "swift-hint", true);
    gtk_label_set_xalign(GTK_LABEL(layoutDesc), 0.0f);
    gtk_label_set_line_wrap(GTK_LABEL(layoutDesc), TRUE);
    gtk_box_pack_start(GTK_BOX(tabAppearance), layoutDesc, FALSE, FALSE, 0);

    GtkWidget* layoutGrid = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    auto* layoutCards = new std::vector<GtkWidget*>();

    const std::array<std::tuple<SnUiLayout, const char*, const char*>, 3> layouts = {{
            {SN_LAYOUT_PURE_TOP_BAR, "B1 · Pure Top Bar", "Thanh mảnh 40px kiểu Apple Notes"},
            {SN_LAYOUT_CANVAS_PILL, "B2 · Floating Pill", "3 viên thuốc sát mép kiểu Freeform"},
            {SN_LAYOUT_ZEN_BOTTOM_DOCK, "B3 · Zen Bottom", "Dải bút gọn sát cạnh dưới"},
    }};

    for (const auto& [mode, title, sub]: layouts) {
        GtkWidget* card = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(card), GTK_RELIEF_NONE);
        setCssClass(card, "swift-layout-card", true);
        setCssClass(card, "swift-active", this->settings.uiLayout == mode);

        GtkWidget* vb = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
        GtkWidget* thumb = gtk_drawing_area_new();
        gtk_widget_set_size_request(thumb, 128, 76);
        g_signal_connect(thumb, "draw", G_CALLBACK(drawLayoutThumb), GINT_TO_POINTER(static_cast<int>(mode)));
        gtk_box_pack_start(GTK_BOX(vb), thumb, FALSE, FALSE, 0);

        GtkWidget* tl = gtk_label_new(title);
        gtk_label_set_xalign(GTK_LABEL(tl), 0.0f);
        gtk_box_pack_start(GTK_BOX(vb), tl, FALSE, FALSE, 0);

        GtkWidget* sl = gtk_label_new(sub);
        setCssClass(sl, "swift-hint", true);
        gtk_label_set_xalign(GTK_LABEL(sl), 0.0f);
        gtk_label_set_line_wrap(GTK_LABEL(sl), TRUE);
        gtk_box_pack_start(GTK_BOX(vb), sl, FALSE, FALSE, 0);

        gtk_container_add(GTK_CONTAINER(card), vb);
        g_object_set_data(G_OBJECT(card), "layout-mode", GINT_TO_POINTER(static_cast<int>(mode)));
        g_object_set_data(G_OBJECT(card), "layout-cards", layoutCards);
        g_signal_connect(card, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             const auto m = static_cast<SnUiLayout>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "layout-mode")));
                             self->applyLayoutMode(m);
                             SwiftBridge::setSettings(self->settings);
                             SwiftBridge::saveState();
                             if (auto* cards = static_cast<std::vector<GtkWidget*>*>(
                                         g_object_get_data(G_OBJECT(b), "layout-cards"))) {
                                 for (GtkWidget* c: *cards) {
                                     setCssClass(c, "swift-active", c == GTK_WIDGET(b));
                                 }
                             }
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(layoutGrid), card, TRUE, TRUE, 0);
        layoutCards->push_back(card);
    }
    gtk_box_pack_start(GTK_BOX(tabAppearance), layoutGrid, FALSE, FALSE, 0);

    gtk_box_pack_start(GTK_BOX(tabAppearance), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL), FALSE, FALSE, 4);

    GtkWidget* themeLbl = gtk_label_new("Chế độ màu (100% bề mặt đặc, không kính mờ)");
    gtk_label_set_xalign(GTK_LABEL(themeLbl), 0.0f);
    gtk_box_pack_start(GTK_BOX(tabAppearance), themeLbl, FALSE, FALSE, 0);

    GtkWidget* themeSeg = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    setCssClass(themeSeg, "swift-segmented", true);
    auto* themeBtns = new std::vector<GtkWidget*>();
    const std::array<std::pair<const char*, SnThemeMode>, 3> themes = {{
            {"Sáng (Light)", SN_THEME_LIGHT},
            {"Tối (Dark)", SN_THEME_DARK},
            {"Theo hệ thống", SN_THEME_SYSTEM},
    }};
    for (const auto& [lbl, tm]: themes) {
        GtkWidget* b = gtk_button_new_with_label(lbl);
        setCssClass(b, "swift-active", this->settings.theme == tm);
        g_object_set_data(G_OBJECT(b), "theme-mode", GINT_TO_POINTER(static_cast<int>(tm)));
        g_object_set_data(G_OBJECT(b), "dialog", dlg);
        g_object_set_data(G_OBJECT(b), "theme-btns", themeBtns);
        g_signal_connect(b, "clicked", G_CALLBACK(+[](GtkButton* btn, gpointer selfPtr) {
                             auto* self = static_cast<SwiftShell*>(selfPtr);
                             self->settings.theme = static_cast<SnThemeMode>(
                                     GPOINTER_TO_INT(g_object_get_data(G_OBJECT(btn), "theme-mode")));
                             SwiftBridge::setSettings(self->settings);
                             SwiftBridge::saveState();
                             self->applyThemeFromSettings();
                             GtkWidget* d = GTK_WIDGET(g_object_get_data(G_OBJECT(btn), "dialog"));
                             setCssClass(d, "darkMode", self->win && self->win->isDarkTheme());
                             if (auto* list = static_cast<std::vector<GtkWidget*>*>(
                                         g_object_get_data(G_OBJECT(btn), "theme-btns"))) {
                                 for (GtkWidget* tb: *list) {
                                     setCssClass(tb, "swift-active", tb == GTK_WIDGET(btn));
                                 }
                             }
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(themeSeg), b, TRUE, TRUE, 0);
        themeBtns->push_back(b);
    }
    gtk_box_pack_start(GTK_BOX(tabAppearance), themeSeg, FALSE, FALSE, 0);
    gtk_stack_add_named(GTK_STACK(tabStack), tabAppearance, "appearance");

    // =========================================================================
    // TAB 2: BẢNG ĐEN & GIẤY (Live Cairo Preview)
    // =========================================================================
    GtkWidget* tabBlackboard = gtk_box_new(GTK_ORIENTATION_VERTICAL, 14);
    GtkWidget* h2 = gtk_label_new("Chế độ Bảng đen & Mẫu giấy");
    setCssClass(h2, "swift-settings-heading", true);
    gtk_label_set_xalign(GTK_LABEL(h2), 0.0f);
    gtk_box_pack_start(GTK_BOX(tabBlackboard), h2, FALSE, FALSE, 0);

    GtkWidget* bbPreview = gtk_drawing_area_new();
    gtk_widget_set_size_request(bbPreview, -1, 74);
    g_signal_connect(bbPreview, "draw", G_CALLBACK(drawBlackboardLivePreview), nullptr);
    gtk_box_pack_start(GTK_BOX(tabBlackboard), bbPreview, FALSE, FALSE, 0);

    auto addSwitch = [&](GtkWidget* parentBox, const char* titleText, const char* descText, bool initVal,
                         GtkWidget* redrawTarget, GCallback cb) {
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
        GtkWidget* tb = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
        GtkWidget* l = gtk_label_new(titleText);
        gtk_label_set_xalign(GTK_LABEL(l), 0.0f);
        gtk_box_pack_start(GTK_BOX(tb), l, FALSE, FALSE, 0);
        if (descText) {
            GtkWidget* sub = gtk_label_new(descText);
            setCssClass(sub, "swift-hint", true);
            gtk_label_set_xalign(GTK_LABEL(sub), 0.0f);
            gtk_label_set_line_wrap(GTK_LABEL(sub), TRUE);
            gtk_box_pack_start(GTK_BOX(tb), sub, FALSE, FALSE, 0);
        }
        GtkWidget* sw = gtk_switch_new();
        gtk_switch_set_active(GTK_SWITCH(sw), initVal);
        gtk_widget_set_valign(sw, GTK_ALIGN_CENTER);
        if (redrawTarget) {
            g_object_set_data(G_OBJECT(sw), "redraw-target", redrawTarget);
        }
        g_signal_connect(sw, "state-set", cb, this);
        gtk_box_pack_start(GTK_BOX(row), tb, TRUE, TRUE, 0);
        gtk_box_pack_end(GTK_BOX(row), sw, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(parentBox), row, FALSE, FALSE, 0);
    };

    addSwitch(tabBlackboard, "Bật chế độ Bảng đen (Blackboard)",
              "Hiển thị nền bảng đen dịu mắt mà không thay đổi nền gốc trong file .xopp.",
              this->settings.blackboardEnabled, bbPreview,
              G_CALLBACK(+[](GtkSwitch* sw, gboolean state, gpointer selfPtr) -> gboolean {
                  auto* self = static_cast<SwiftShell*>(selfPtr);
                  self->settings.blackboardEnabled = (state != FALSE);
                  SwiftBridge::setSettings(self->settings);
                  SwiftBridge::saveState();
                  self->refreshCanvasForDisplayState();
                  self->rebuildPenSlotButtons();
                  self->syncToolHighlight();
                  if (auto* rt = GTK_WIDGET(g_object_get_data(G_OBJECT(sw), "redraw-target"))) {
                      gtk_widget_queue_draw(rt);
                  }
                  return FALSE;
              }));

    addSwitch(tabBlackboard, "Tự động đảo màu mực đen ↔ trắng",
              "Nét và chữ màu đen tự hiển thị thành trắng trên Bảng đen; màu đỏ, xanh, vàng giữ nguyên.",
              this->settings.blackboardAutoInvertBwInk, bbPreview,
              G_CALLBACK(+[](GtkSwitch* sw, gboolean state, gpointer selfPtr) -> gboolean {
                  auto* self = static_cast<SwiftShell*>(selfPtr);
                  self->settings.blackboardAutoInvertBwInk = (state != FALSE);
                  SwiftBridge::setSettings(self->settings);
                  SwiftBridge::saveState();
                  self->refreshCanvasForDisplayState();
                  self->rebuildPenSlotButtons();
                  if (auto* rt = GTK_WIDGET(g_object_get_data(G_OBJECT(sw), "redraw-target"))) {
                      gtk_widget_queue_draw(rt);
                  }
                  return FALSE;
              }));

    gtk_stack_add_named(GTK_STACK(tabStack), tabBlackboard, "blackboard");

    // =========================================================================
    // TAB 3: BÚT, TẨY & PHÓNG TO
    // =========================================================================
    GtkWidget* tabPen = gtk_box_new(GTK_ORIENTATION_VERTICAL, 14);
    GtkWidget* h3 = gtk_label_new("Bút, Tẩy & Vùng phóng to");
    setCssClass(h3, "swift-settings-heading", true);
    gtk_label_set_xalign(GTK_LABEL(h3), 0.0f);
    gtk_box_pack_start(GTK_BOX(tabPen), h3, FALSE, FALSE, 0);

    addSwitch(tabPen, "Tự quay lại bút sau khi tẩy",
              "Hoàn tất nét tẩy và nhấc bút ra sẽ tự chuyển về đúng slot bút trước đó.",
              this->settings.eraserAutoReturn, nullptr,
              G_CALLBACK(+[](GtkSwitch*, gboolean state, gpointer selfPtr) -> gboolean {
                  auto* self = static_cast<SwiftShell*>(selfPtr);
                  self->settings.eraserAutoReturn = (state != FALSE);
                  SwiftBridge::setSettings(self->settings);
                  SwiftBridge::saveState();
                  return FALSE;
              }));

    addSwitch(tabPen, "Tự trượt khung vùng viết phóng to (Close-up)",
              "Khi viết tới mép phải của khung phóng to, khung tự trượt mượt sang phải hoặc xuống dòng.",
              this->settings.closeUpAutoAdvance, nullptr,
              G_CALLBACK(+[](GtkSwitch*, gboolean state, gpointer selfPtr) -> gboolean {
                  auto* self = static_cast<SwiftShell*>(selfPtr);
                  self->settings.closeUpAutoAdvance = (state != FALSE);
                  SwiftBridge::setSettings(self->settings);
                  SwiftBridge::saveState();
                  return FALSE;
              }));

    gtk_box_pack_start(GTK_BOX(tabPen), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL), FALSE, FALSE, 4);

    GtkWidget* advBtn = gtk_button_new_with_label("Mở cài đặt bút vẽ, bảng vẽ & phần cứng chuyên sâu...");
    setCssClass(advBtn, "swift-secondary-btn", true);
    g_object_set_data(G_OBJECT(advBtn), "dialog", dlg);
    g_signal_connect(advBtn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftShell*>(selfPtr);
                         GtkWidget* d = GTK_WIDGET(g_object_get_data(G_OBJECT(b), "dialog"));
                         gtk_dialog_response(GTK_DIALOG(d), GTK_RESPONSE_CLOSE);
                         self->control->showSettings();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(tabPen), advBtn, FALSE, FALSE, 0);
    gtk_stack_add_named(GTK_STACK(tabStack), tabPen, "pen");

    // =========================================================================
    // TAB 4: PHÍM TẮT (Visual Interactive Keycap Customizer)
    // =========================================================================
    GtkWidget* tabShortcuts = gtk_box_new(GTK_ORIENTATION_VERTICAL, 10);
    GtkWidget* scHeader = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    GtkWidget* h4 = gtk_label_new("Phím tắt đơn giản (kiểu Scrble Ink)");
    setCssClass(h4, "swift-settings-heading", true);
    gtk_label_set_xalign(GTK_LABEL(h4), 0.0f);
    gtk_box_pack_start(GTK_BOX(scHeader), h4, TRUE, TRUE, 0);

    auto* recorder = new ShortcutRecorderState{this, nullptr, {}, {}};

    GtkWidget* resetBtn = gtk_button_new_with_label("Khôi phục mặc định");
    setCssClass(resetBtn, "swift-secondary-btn", true);
    g_signal_connect(resetBtn, "clicked", G_CALLBACK(+[](GtkButton*, gpointer recPtr) {
                         auto* rec = static_cast<ShortcutRecorderState*>(recPtr);
                         SwiftBridge::resetShortcuts();
                         SwiftBridge::saveState();
                         rec->shell->shortcuts = SwiftBridge::getShortcuts();
                         rec->activeButton = nullptr;
                         rec->activeActionId.clear();
                         for (const auto& [aid, btn]: rec->keyButtons) {
                             setCssClass(btn, "swift-recording", false);
                             gtk_button_set_label(GTK_BUTTON(btn), rec->shell->getShortcutKeyDisplay(aid).c_str());
                         }
                         rec->shell->refreshTooltipsWithShortcuts();
                         rec->shell->rebuildPenSlotButtons();
                     }),
                     recorder);
    gtk_box_pack_end(GTK_BOX(scHeader), resetBtn, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(tabShortcuts), scHeader, FALSE, FALSE, 0);

    GtkWidget* scHint = gtk_label_new(
            "Bấm vào ô phím để gán phím mới (A–Z, 0–9, Tab). Nhấn Backspace để bỏ gán, Esc để hủy. Phím tắt tự tạm dừng khi đang gõ Text Box.");
    setCssClass(scHint, "swift-hint", true);
    gtk_label_set_xalign(GTK_LABEL(scHint), 0.0f);
    gtk_label_set_line_wrap(GTK_LABEL(scHint), TRUE);
    gtk_box_pack_start(GTK_BOX(tabShortcuts), scHint, FALSE, FALSE, 0);

    GtkWidget* scScroll = gtk_scrolled_window_new(nullptr, nullptr);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scScroll), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    GtkWidget* scList = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);

    for (const auto& sc: this->shortcuts) {
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
        setCssClass(row, "swift-shortcut-row", true);

        GtkWidget* lbl = gtk_label_new(sc.label.c_str());
        gtk_label_set_xalign(GTK_LABEL(lbl), 0.0f);
        gtk_box_pack_start(GTK_BOX(row), lbl, TRUE, TRUE, 0);

        GtkWidget* keyBtn = gtk_button_new_with_label(formatKeyUpper(sc.key).c_str());
        setCssClass(keyBtn, "swift-keycap-btn", true);
        g_object_set_data_full(G_OBJECT(keyBtn), "action-id", g_strdup(sc.actionId.c_str()), g_free);
        g_signal_connect(keyBtn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer recPtr) {
                             auto* rec = static_cast<ShortcutRecorderState*>(recPtr);
                             if (rec->activeButton && rec->activeButton != GTK_WIDGET(b)) {
                                 setCssClass(rec->activeButton, "swift-recording", false);
                                 gtk_button_set_label(GTK_BUTTON(rec->activeButton),
                                                      rec->shell->getShortcutKeyDisplay(rec->activeActionId).c_str());
                             }
                             rec->activeButton = GTK_WIDGET(b);
                             const char* aid = static_cast<const char*>(g_object_get_data(G_OBJECT(b), "action-id"));
                             rec->activeActionId = aid ? aid : "";
                             setCssClass(GTK_WIDGET(b), "swift-recording", true);
                             gtk_button_set_label(b, "Nhấn phím...");
                         }),
                         recorder);
        gtk_box_pack_end(GTK_BOX(row), keyBtn, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(scList), row, FALSE, FALSE, 0);
        recorder->keyButtons.emplace_back(sc.actionId, keyBtn);
    }

    gtk_container_add(GTK_CONTAINER(scScroll), scList);
    gtk_box_pack_start(GTK_BOX(tabShortcuts), scScroll, TRUE, TRUE, 0);
    gtk_stack_add_named(GTK_STACK(tabStack), tabShortcuts, "shortcuts");

    // Capture keystrokes while recording a new shortcut
    g_signal_connect(dlg, "key-press-event",
                     G_CALLBACK(+[](GtkWidget*, GdkEventKey* ev, gpointer recPtr) -> gboolean {
                         auto* rec = static_cast<ShortcutRecorderState*>(recPtr);
                         if (!rec->activeButton || rec->activeActionId.empty()) {
                             return FALSE;
                         }
                         if (ev->keyval == GDK_KEY_Escape) {
                             setCssClass(rec->activeButton, "swift-recording", false);
                             gtk_button_set_label(GTK_BUTTON(rec->activeButton),
                                                  rec->shell->getShortcutKeyDisplay(rec->activeActionId).c_str());
                             rec->activeButton = nullptr;
                             rec->activeActionId.clear();
                             return TRUE;
                         }
                         std::string newKey;
                         if (ev->keyval == GDK_KEY_BackSpace || ev->keyval == GDK_KEY_Delete) {
                             newKey.clear();
                         } else {
                             newKey = normalizeGdkKeyval(ev->keyval);
                             if (newKey.empty()) {
                                 return TRUE;
                             }
                         }
                         SwiftBridge::setShortcutKey(rec->activeActionId, newKey);
                         SwiftBridge::saveState();
                         rec->shell->shortcuts = SwiftBridge::getShortcuts();

                         // Update all keycap buttons (in case a conflicting key was cleared)
                         for (const auto& [aid, btn]: rec->keyButtons) {
                             setCssClass(btn, "swift-recording", false);
                             gtk_button_set_label(GTK_BUTTON(btn), rec->shell->getShortcutKeyDisplay(aid).c_str());
                         }
                         rec->activeButton = nullptr;
                         rec->activeActionId.clear();
                         rec->shell->refreshTooltipsWithShortcuts();
                         rec->shell->rebuildPenSlotButtons();
                         return TRUE;
                     }),
                     recorder);

    gtk_box_pack_start(GTK_BOX(split), sidebar, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(split), tabStack, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(contentArea), split, TRUE, TRUE, 0);

    gtk_widget_show_all(dlg);
    gtk_dialog_run(GTK_DIALOG(dlg));
    gtk_widget_destroy(dlg);

    delete tabButtons;
    delete layoutCards;
    delete themeBtns;
    delete recorder;
}

}  // namespace swiftnote

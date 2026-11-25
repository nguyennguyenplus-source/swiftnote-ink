#include "SwiftBridge.h"

#include <algorithm>
#include <cstring>
#include <mutex>

namespace swiftnote {

namespace {

std::atomic<bool> gBlackboardEnabled{false};
std::atomic<bool> gAutoInvertBwInk{true};
std::atomic<uint32_t> gBlackboardSurfaceRgb{0x16181DU};

thread_local std::optional<bool> tlBlackboardOverride;
thread_local std::optional<bool> tlInvertOverride;

std::mutex gEraseCbMutex;
std::function<void()> gEraseFinishedCallback;

std::vector<uint8_t> takeOwnedBuffer(SnByteBuffer buf) {
    std::vector<uint8_t> out;
    if (buf.data && buf.len > 0) {
        out.assign(buf.data, buf.data + buf.len);
    }
    sn_buffer_free(buf);
    return out;
}

template <size_t N>
void copyToFixed(char (&dst)[N], const std::string& src) {
    std::memset(dst, 0, N);
    if (!src.empty()) {
        std::memcpy(dst, src.data(), std::min(src.size(), N - 1));
    }
}

PenSlot fromFfiSlot(const SnPenSlot& s) {
    PenSlot out;
    out.id = s.id;
    out.label = s.label;
    out.kind = s.kind;
    out.color = Color{0xFF000000U | (s.color_rgb & 0x00FFFFFFU)};
    out.widthPt = s.width_pt;
    out.pressureSensitive = s.pressure_sensitive;
    out.lineStyle = s.line_style;
    out.inkToShape = s.ink_to_shape;
    return out;
}

SnPenSlot toFfiSlot(const PenSlot& s) {
    SnPenSlot out{};
    out.id = s.id;
    copyToFixed(out.label, s.label);
    out.kind = s.kind;
    out.color_rgb = uint32_t(s.color) & 0x00FFFFFFU;
    out.width_pt = s.widthPt;
    out.pressure_sensitive = s.pressureSensitive;
    out.line_style = s.lineStyle;
    out.ink_to_shape = s.inkToShape;
    return out;
}

}  // namespace

bool DisplayState::isBlackboardActive() {
    if (tlBlackboardOverride.has_value()) {
        return *tlBlackboardOverride;
    }
    return gBlackboardEnabled.load(std::memory_order_relaxed);
}

bool DisplayState::isAutoInvertBwInkEnabled() {
    if (tlInvertOverride.has_value()) {
        return *tlInvertOverride;
    }
    return gAutoInvertBwInk.load(std::memory_order_relaxed);
}

Color DisplayState::blackboardSurfaceColor() {
    return Color{0xFF000000U | (gBlackboardSurfaceRgb.load(std::memory_order_relaxed) & 0x00FFFFFFU)};
}

void DisplayState::setBlackboardActive(bool enabled) {
    gBlackboardEnabled.store(enabled, std::memory_order_relaxed);
}

void DisplayState::setAutoInvertBwInk(bool enabled) {
    gAutoInvertBwInk.store(enabled, std::memory_order_relaxed);
}

void DisplayState::setBlackboardSurfaceColor(Color color) {
    gBlackboardSurfaceRgb.store(uint32_t(color) & 0x00FFFFFFU, std::memory_order_relaxed);
}

Color DisplayState::resolveInkColor(Color original) {
    if (original.alpha == 0) {
        original.alpha = 0xFFU;
    }
    if (!isBlackboardActive() || !isAutoInvertBwInkEnabled()) {
        return original;
    }
    Color inverted = SwiftBridge::invertBlackWhiteInk(original);
    inverted.alpha = original.alpha;
    return inverted;
}

DisplayState::ScopedExportOverride::ScopedExportOverride(bool keepBlackboard, bool keepInkInvert):
        prevBlackboard(tlBlackboardOverride.value_or(gBlackboardEnabled.load(std::memory_order_relaxed))),
        prevInvert(tlInvertOverride.value_or(gAutoInvertBwInk.load(std::memory_order_relaxed))) {
    tlBlackboardOverride = keepBlackboard;
    tlInvertOverride = keepInkInvert;
}

DisplayState::ScopedExportOverride::~ScopedExportOverride() {
    tlBlackboardOverride.reset();
    tlInvertOverride.reset();
}

void registerEraseFinishedCallback(std::function<void()> cb) {
    std::lock_guard<std::mutex> lock(gEraseCbMutex);
    gEraseFinishedCallback = std::move(cb);
}

void notifyEraseStrokeFinished() {
    std::function<void()> cb;
    {
        std::lock_guard<std::mutex> lock(gEraseCbMutex);
        cb = gEraseFinishedCallback;
    }
    if (cb) {
        cb();
    }
}

std::string SwiftBridge::version() {
    const char* v = sn_version();
    return v ? std::string(v) : std::string("0.0.0");
}

bool SwiftBridge::initState(const std::string& path) {
    const char* cpath = path.empty() ? nullptr : path.c_str();
    if (sn_state_init(cpath) != SN_OK) {
        return false;
    }
    auto s = getSettings();
    DisplayState::setBlackboardActive(s.blackboardEnabled);
    DisplayState::setAutoInvertBwInk(s.blackboardAutoInvertBwInk);
    DisplayState::setBlackboardSurfaceColor(s.blackboardSurfaceColor);
    return true;
}

AppSettings SwiftBridge::getSettings() {
    SnAppSettings raw{};
    AppSettings out;
    if (sn_state_get_settings(&raw) != SN_OK) {
        return out;
    }
    out.theme = raw.theme;
    out.uiLayout = raw.ui_layout;
    out.activePenSlot = raw.active_pen_slot;
    out.penSlotCount = raw.pen_slot_count;
    out.eraserAutoReturn = raw.eraser_auto_return;
    out.blackboardEnabled = raw.blackboard_enabled;
    out.blackboardAutoInvertBwInk = raw.blackboard_auto_invert_bw_ink;
    out.blackboardSurfaceColor = Color{0xFF000000U | (raw.blackboard_surface_rgb & 0x00FFFFFFU)};
    out.closeUpZoomFactor = raw.close_up_zoom_factor;
    out.closeUpAutoAdvance = raw.close_up_auto_advance;
    out.closeUpPanelHeightPx = raw.close_up_panel_height_px;
    out.closeUpMarginTriggerRatio = raw.close_up_margin_trigger_ratio;
    out.libraryDir = raw.library_dir;
    out.defaultPaperTemplate = raw.default_paper_template;
    return out;
}

bool SwiftBridge::setSettings(const AppSettings& s) {
    SnAppSettings raw{};
    raw.theme = s.theme;
    raw.ui_layout = s.uiLayout;
    raw.active_pen_slot = s.activePenSlot;
    raw.pen_slot_count = s.penSlotCount;
    raw.eraser_auto_return = s.eraserAutoReturn;
    raw.blackboard_enabled = s.blackboardEnabled;
    raw.blackboard_auto_invert_bw_ink = s.blackboardAutoInvertBwInk;
    raw.blackboard_surface_rgb = uint32_t(s.blackboardSurfaceColor) & 0x00FFFFFFU;
    raw.close_up_zoom_factor = s.closeUpZoomFactor;
    raw.close_up_auto_advance = s.closeUpAutoAdvance;
    raw.close_up_panel_height_px = s.closeUpPanelHeightPx;
    raw.close_up_margin_trigger_ratio = s.closeUpMarginTriggerRatio;
    copyToFixed(raw.library_dir, s.libraryDir);
    copyToFixed(raw.default_paper_template, s.defaultPaperTemplate);

    if (sn_state_set_settings(&raw) != SN_OK) {
        return false;
    }
    DisplayState::setBlackboardActive(s.blackboardEnabled);
    DisplayState::setAutoInvertBwInk(s.blackboardAutoInvertBwInk);
    DisplayState::setBlackboardSurfaceColor(s.blackboardSurfaceColor);
    return true;
}

std::vector<PenSlot> SwiftBridge::getPenSlots() {
    std::vector<PenSlot> out;
    const uint32_t count = sn_state_pen_slot_count();
    out.reserve(count);
    for (uint32_t i = 0; i < count; ++i) {
        SnPenSlot raw{};
        if (sn_state_get_pen_slot(i, &raw) == SN_OK) {
            out.push_back(fromFfiSlot(raw));
        }
    }
    return out;
}

bool SwiftBridge::setPenSlot(uint32_t index, const PenSlot& slot) {
    SnPenSlot raw = toFfiSlot(slot);
    return sn_state_set_pen_slot(index, &raw) == SN_OK;
}

std::optional<uint32_t> SwiftBridge::addPenSlot(const PenSlot& slot) {
    SnPenSlot raw = toFfiSlot(slot);
    uint32_t idx = 0;
    if (sn_state_add_pen_slot(&raw, &idx) != SN_OK) {
        return std::nullopt;
    }
    return idx;
}

bool SwiftBridge::removePenSlot(uint32_t index) { return sn_state_remove_pen_slot(index) == SN_OK; }

std::vector<ShortcutEntry> SwiftBridge::getShortcuts() {
    std::vector<ShortcutEntry> out;
    const uint32_t count = sn_state_shortcut_count();
    out.reserve(count);
    for (uint32_t i = 0; i < count; ++i) {
        SnShortcutEntry raw{};
        if (sn_state_get_shortcut(i, &raw) == SN_OK) {
            out.push_back(ShortcutEntry{raw.action_id, raw.label, raw.key});
        }
    }
    return out;
}

bool SwiftBridge::setShortcutKey(const std::string& actionId, const std::string& key) {
    return sn_state_set_shortcut_key(actionId.c_str(), key.c_str()) == SN_OK;
}

bool SwiftBridge::resetShortcuts() { return sn_state_reset_shortcuts() == SN_OK; }

bool SwiftBridge::saveState() { return sn_state_save() == SN_OK; }

Color SwiftBridge::invertBlackWhiteInk(Color color) {
    const uint32_t raw = uint32_t(color);
    const uint32_t alpha = (raw & 0xFF000000U) == 0 ? 0xFF000000U : (raw & 0xFF000000U);
    const uint32_t rgb = raw & 0x00FFFFFFU;
    const uint32_t inverted = sn_invert_bw_ink_rgb(rgb);
    return Color{alpha | (inverted & 0x00FFFFFFU)};
}

std::optional<NormalizedPng> SwiftBridge::normalizeClipboardPng(const uint8_t* data, size_t len) {
    if (!data || len == 0) {
        return std::nullopt;
    }
    SnByteBuffer outBuf{nullptr, 0, 0};
    uint32_t w = 0;
    uint32_t h = 0;
    if (sn_normalize_clipboard_png(data, len, &outBuf, &w, &h) != SN_OK) {
        return std::nullopt;
    }
    NormalizedPng res;
    res.bytes = takeOwnedBuffer(outBuf);
    res.width = w;
    res.height = h;
    return res;
}

std::optional<std::vector<uint8_t>> SwiftBridge::pixelsToLosslessPng(const uint8_t* pixels,
                                                                     size_t len,
                                                                     uint32_t width,
                                                                     uint32_t height,
                                                                     uint32_t stride,
                                                                     bool isBgra,
                                                                     bool fixAllZeroAlpha) {
    if (!pixels || len == 0) {
        return std::nullopt;
    }
    SnByteBuffer outBuf{nullptr, 0, 0};
    if (sn_pixels_to_lossless_png(pixels, len, width, height, stride, isBgra, fixAllZeroAlpha, &outBuf) != SN_OK) {
        return std::nullopt;
    }
    return takeOwnedBuffer(outBuf);
}

std::vector<NoteItem> SwiftBridge::scanLibrary(const std::string& rootDir, const std::string& query) {
    std::vector<NoteItem> out;
    SnNoteList list{nullptr, 0, 0};
    const char* q = query.empty() ? nullptr : query.c_str();
    if (sn_library_scan_list(rootDir.c_str(), q, &list) != SN_OK) {
        return out;
    }
    out.reserve(list.len);
    for (size_t i = 0; i < list.len; ++i) {
        const auto& src = list.items[i];
        NoteItem item;
        item.title = src.title;
        item.path = src.path;
        item.folder = src.folder;
        item.kind = src.kind;
        item.modifiedEpochSecs = src.modified_epoch_secs;
        item.sizeBytes = src.size_bytes;
        item.pageCount = src.page_count;
        item.hasPreview = src.has_preview;
        out.push_back(std::move(item));
    }
    sn_note_list_free(list);
    return out;
}

std::optional<std::vector<uint8_t>> SwiftBridge::extractXoppPreview(const std::string& filePath) {
    SnByteBuffer outBuf{nullptr, 0, 0};
    if (sn_library_extract_xopp_preview(filePath.c_str(), &outBuf) != SN_OK) {
        return std::nullopt;
    }
    return takeOwnedBuffer(outBuf);
}

}  // namespace swiftnote

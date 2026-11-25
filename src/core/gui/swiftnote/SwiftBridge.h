#pragma once

#include <atomic>
#include <cstdint>
#include <functional>
#include <optional>
#include <string>
#include <vector>

#include "swiftnote_ffi.h"
#include "util/Color.h"

namespace swiftnote {

struct NormalizedPng {
    std::vector<uint8_t> bytes;
    uint32_t width = 0;
    uint32_t height = 0;
};

struct PenSlot {
    uint32_t id = 1;
    std::string label;
    SnPenKind kind = SN_PEN_KIND_PEN;
    Color color{0x111111U};
    double widthPt = 1.41;
    bool pressureSensitive = true;
    SnLineStyle lineStyle = SN_LINE_STYLE_SOLID;
    bool inkToShape = false;
};

struct ShortcutEntry {
    std::string actionId;
    std::string label;
    std::string key;
};

struct AppSettings {
    SnThemeMode theme = SN_THEME_LIGHT;
    SnUiLayout uiLayout = SN_LAYOUT_CANVAS_PILL;
    uint32_t activePenSlot = 0;
    uint32_t penSlotCount = 5;
    bool eraserAutoReturn = true;
    bool blackboardEnabled = false;
    bool blackboardAutoInvertBwInk = true;
    Color blackboardSurfaceColor{0x16181DU};
    double closeUpZoomFactor = 2.5;
    bool closeUpAutoAdvance = true;
    uint32_t closeUpPanelHeightPx = 260;
    double closeUpMarginTriggerRatio = 0.78;
    std::string libraryDir;
    std::string defaultPaperTemplate = "ruled";
};

struct NoteItem {
    std::string title;
    std::string path;
    std::string folder;
    SnNoteKind kind = SN_NOTE_KIND_XOPP;
    uint64_t modifiedEpochSecs = 0;
    uint64_t sizeBytes = 0;
    uint32_t pageCount = 0;
    bool hasPreview = false;
};

/**
 * Lock-free display-layer state read on the Cairo rendering hot path.
 * Zero mutex overhead, zero per-point FFI overhead when Blackboard is inactive.
 */
class DisplayState {
public:
    static bool isBlackboardActive();
    static bool isAutoInvertBwInkEnabled();
    static Color blackboardSurfaceColor();

    static void setBlackboardActive(bool enabled);
    static void setAutoInvertBwInk(bool enabled);
    static void setBlackboardSurfaceColor(Color color);

    /**
     * Resolves display color for strokes and text.
     * Inverts black/dark-gray <-> white/light-gray only when Blackboard & auto-invert are enabled.
     */
    static Color resolveInkColor(Color original);

    /**
     * RAII guard to temporarily override Blackboard state during PDF/PNG export
     * (e.g., exporting white printable pages even while viewing in Blackboard mode).
     */
    class ScopedExportOverride {
    public:
        ScopedExportOverride(bool keepBlackboard, bool keepInkInvert);
        ~ScopedExportOverride();
        ScopedExportOverride(const ScopedExportOverride&) = delete;
        ScopedExportOverride& operator=(const ScopedExportOverride&) = delete;

    private:
        bool prevBlackboard;
        bool prevInvert;
    };
};

/**
 * Lightweight hook for EraseHandler::finalize() -> SwiftController auto-return.
 */
void registerEraseFinishedCallback(std::function<void()> cb);
void notifyEraseStrokeFinished();

/**
 * RAII C++ wrapper over the Rust swiftnote-rs C ABI (swiftnote_ffi.h).
 */
class SwiftBridge {
public:
    static std::string version();

    static bool initState(const std::string& path = {});
    static AppSettings getSettings();
    static bool setSettings(const AppSettings& settings);
    static std::vector<PenSlot> getPenSlots();
    static bool setPenSlot(uint32_t index, const PenSlot& slot);
    static std::optional<uint32_t> addPenSlot(const PenSlot& slot);
    static bool removePenSlot(uint32_t index);
    static std::vector<ShortcutEntry> getShortcuts();
    static bool setShortcutKey(const std::string& actionId, const std::string& key);
    static bool resetShortcuts();
    static bool saveState();

    static Color invertBlackWhiteInk(Color color);

    static std::optional<NormalizedPng> normalizeClipboardPng(const uint8_t* data, size_t len);
    static std::optional<std::vector<uint8_t>> pixelsToLosslessPng(const uint8_t* pixels,
                                                                   size_t len,
                                                                   uint32_t width,
                                                                   uint32_t height,
                                                                   uint32_t stride,
                                                                   bool isBgra,
                                                                   bool fixAllZeroAlpha);

    static std::vector<NoteItem> scanLibrary(const std::string& rootDir, const std::string& query = {});
    static std::optional<std::vector<uint8_t>> extractXoppPreview(const std::string& filePath);
};

}  // namespace swiftnote

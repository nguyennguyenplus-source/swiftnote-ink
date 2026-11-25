#pragma once

#include <cstddef>
#include <memory>
#include <string>
#include <vector>

#include <gtk/gtk.h>

#include "control/ToolEnums.h"
#include "gui/swiftnote/SwiftBridge.h"

class Control;
class MainWindow;

namespace swiftnote {

class SwiftLibraryView;
class CloseUpWritingPanel;

/**
 * Swift note's Apple-grade minimalist shell mounted inside MainWindow.
 * - Zero widget duplication across all 3 switchable layouts:
 *     B1: Pure Top Bar (Apple Notes 40px flush bar)
 *     B2: Canvas-First Floating Pill (Apple Freeform, tight to top edge)
 *     B3: Zen Header + Bottom Ink Dock (Craft / Keynote focus)
 * - Scrble Ink-style single-key shortcuts (suppressed during text input)
 * - Visual 4-tab macOS-style Settings dialog with live layout/ink previews & keycap rebinding
 */
class SwiftShell {
public:
    SwiftShell(MainWindow* win, Control* control);
    ~SwiftShell();

    void mount(GtkOverlay* mainOverlay, GtkWidget* mainBox);

    void updateDocumentTitle(const std::string& title);
    void updatePageInfo(size_t currentPageZeroBased, size_t pageCount);
    void syncToolHighlight();
    void applyThemeFromSettings();
    void applyLayoutMode(SnUiLayout layout);
    void toggleFocusMode();
    void showEditor();
    void showLibrary();
    void toggleCloseUpPanel();

    void onEraseStrokeFinished();
    void openSettingsDialog();

    /**
     * Handles single-key Scrble Ink-style shortcuts when editing canvas (returns true if consumed).
     */
    bool handleKeyPress(GdkEventKey* ev);

    void applyPaperTemplate(const std::string& templateId, bool allPages = false);

    GtkWidget* getStack() const { return stack; }
    GtkWidget* getEditorShell() const { return editorShell; }
    AppSettings& getSettingsMut() { return settings; }
    const AppSettings& getSettings() const { return settings; }
    std::string getShortcutKeyDisplay(const std::string& actionId) const;

private:
    void loadBundledFonts();
    void buildToolClusters();
    void rebuildPenSlotButtons();
    void refreshTooltipsWithShortcuts();

    void selectPenSlot(uint32_t index);
    void dispatchShortcutAction(const std::string& actionId);

    void openPenSlotPopover(uint32_t index, GtkWidget* anchor);
    void openEraserPopover(GtkWidget* anchor);
    void openSelectionPopover(GtkWidget* anchor);
    void openShapesPopover(GtkWidget* anchor);
    void openTextPopover(GtkWidget* anchor);
    void openRulerPopover(GtkWidget* anchor);
    void openLaserHandPopover(GtkWidget* anchor);
    void openPaperPopover(GtkWidget* anchor);
    void openFileMenuPopover(GtkWidget* anchor);
    void hideLegacyChrome();

    void toggleBlackboardMode();
    void toggleLightDarkTheme();
    void refreshCanvasForDisplayState();
    void applySlotToToolHandler(const PenSlot& slot);

    GtkWidget* makeIconButton(const char* iconName, const char* tooltip);
    GtkWidget* makeMenuRow(const char* iconName, const char* labelText, bool active = false);
    GtkWidget* makePopover(GtkWidget* anchor);
    void closeActivePopover();

private:
    MainWindow* win = nullptr;
    Control* control = nullptr;

    AppSettings settings{};
    std::vector<PenSlot> penSlots;
    std::vector<ShortcutEntry> shortcuts;
    uint32_t lastWritingSlot = 0;
    bool focusModeHidden = false;
    bool fullscreenActive = false;

    GtkWidget* stack = nullptr;
    GtkWidget* editorShell = nullptr;
    GtkWidget* topHost = nullptr;
    GtkWidget* bottomHost = nullptr;

    // Single reusable set of clusters reparented across B1/B2/B3 layouts
    GtkWidget* leftCluster = nullptr;
    GtkWidget* centerCluster = nullptr;
    GtkWidget* rightCluster = nullptr;

    GtkWidget* logoArea = nullptr;
    GtkWidget* penSlotsBox = nullptr;
    GtkWidget* docTitleLabel = nullptr;
    GtkWidget* pageInfoLabel = nullptr;

    std::vector<GtkWidget*> penSlotButtons;
    GtkWidget* btnLibrary = nullptr;
    GtkWidget* btnEraser = nullptr;
    GtkWidget* btnSelection = nullptr;
    GtkWidget* btnShapes = nullptr;
    GtkWidget* btnText = nullptr;
    GtkWidget* btnRuler = nullptr;
    GtkWidget* btnSpacer = nullptr;
    GtkWidget* btnLaserHand = nullptr;
    GtkWidget* btnUndo = nullptr;
    GtkWidget* btnRedo = nullptr;
    GtkWidget* btnBlackboard = nullptr;
    GtkWidget* btnCloseUp = nullptr;
    GtkWidget* btnAddPage = nullptr;
    GtkWidget* btnSidebar = nullptr;
    GtkWidget* btnSettings = nullptr;

    GtkWidget* activePopover = nullptr;

    std::unique_ptr<SwiftLibraryView> libraryView;
    std::unique_ptr<CloseUpWritingPanel> closeUpPanel;
};

}  // namespace swiftnote

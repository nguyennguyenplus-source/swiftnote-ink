#pragma once

#include <string>
#include <vector>

#include <gtk/gtk.h>

#include "gui/swiftnote/SwiftBridge.h"

class Control;

namespace swiftnote {

class SwiftShell;

/**
 * Dedicated Note Library screen powered by Rust swiftnote-library.
 * Features instant Vietnamese diacritic-insensitive search, folder filtering,
 * embedded .xopp thumbnail previews, and 1-click new note creation from paper templates.
 */
class SwiftLibraryView {
public:
    SwiftLibraryView(SwiftShell* shell, Control* control);
    ~SwiftLibraryView();

    GtkWidget* getWidget() const { return rootWidget; }
    void refresh();

private:
    void buildUi();
    void rebuildSidebarFolders(const std::vector<NoteItem>& allNotes);
    void rebuildCards(const std::vector<NoteItem>& items);
    void openNewNotePopover(GtkWidget* anchor);
    void createNoteWithTemplate(const std::string& templateId);
    void chooseLibraryFolder();

    GtkWidget* makeNoteCard(const NoteItem& item);

private:
    SwiftShell* shell = nullptr;
    Control* control = nullptr;

    GtkWidget* rootWidget = nullptr;
    GtkWidget* foldersBox = nullptr;
    GtkWidget* searchEntry = nullptr;
    GtkWidget* statusLabel = nullptr;
    GtkWidget* flowBox = nullptr;
    GtkWidget* activePopover = nullptr;

    std::string activeFolderFilter;  // empty = All
    int kindFilter = -1;             // -1 = All, 0 = xopp/xoj, 2 = pdf
    std::string searchQuery;
};

}  // namespace swiftnote

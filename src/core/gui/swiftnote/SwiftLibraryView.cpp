#include "SwiftLibraryView.h"

#include <algorithm>
#include <array>
#include <cstdio>
#include <ctime>
#include <filesystem>
#include <set>

#include <gdk-pixbuf/gdk-pixbuf.h>

#include "control/Control.h"
#include "gui/MainWindow.h"
#include "gui/swiftnote/SwiftShell.h"

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

std::string formatShortDate(uint64_t epochSecs) {
    if (epochSecs == 0) {
        return "";
    }
    std::time_t t = static_cast<std::time_t>(epochSecs);
    std::tm tmBuf{};
#ifdef _WIN32
    localtime_s(&tmBuf, &t);
#else
    localtime_r(&t, &tmBuf);
#endif
    char out[32];
    std::strftime(out, sizeof(out), "%d/%m/%Y %H:%M", &tmBuf);
    return out;
}

GdkPixbuf* decodePngThumbnail(const std::vector<uint8_t>& bytes, int maxW, int maxH) {
    if (bytes.empty()) {
        return nullptr;
    }
    GdkPixbufLoader* loader = gdk_pixbuf_loader_new();
    GError* err = nullptr;
    if (!gdk_pixbuf_loader_write(loader, bytes.data(), bytes.size(), &err)) {
        if (err) {
            g_error_free(err);
        }
        gdk_pixbuf_loader_close(loader, nullptr);
        g_object_unref(loader);
        return nullptr;
    }
    gdk_pixbuf_loader_close(loader, nullptr);
    GdkPixbuf* raw = gdk_pixbuf_loader_get_pixbuf(loader);
    GdkPixbuf* scaled = nullptr;
    if (raw) {
        const int rw = gdk_pixbuf_get_width(raw);
        const int rh = gdk_pixbuf_get_height(raw);
        if (rw > 0 && rh > 0) {
            const double ratio = std::min(static_cast<double>(maxW) / rw, static_cast<double>(maxH) / rh);
            const int tw = std::max(1, static_cast<int>(rw * ratio));
            const int th = std::max(1, static_cast<int>(rh * ratio));
            scaled = gdk_pixbuf_scale_simple(raw, tw, th, GDK_INTERP_BILINEAR);
        }
    }
    g_object_unref(loader);
    return scaled;
}

}  // namespace

SwiftLibraryView::SwiftLibraryView(SwiftShell* shell, Control* control): shell(shell), control(control) {
    buildUi();
}

SwiftLibraryView::~SwiftLibraryView() {
    if (this->activePopover) {
        g_signal_handlers_disconnect_by_data(this->activePopover, this);
        gtk_widget_destroy(this->activePopover);
        this->activePopover = nullptr;
    }
}

void SwiftLibraryView::buildUi() {
    this->rootWidget = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
    gtk_widget_set_name(this->rootWidget, "swiftLibraryView");

    // === LEFT SIDEBAR ===
    GtkWidget* sidebar = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    gtk_widget_set_name(sidebar, "swiftLibrarySidebar");
    gtk_widget_set_size_request(sidebar, 224, -1);

    GtkWidget* brandRow = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_margin_start(brandRow, 8);
    gtk_widget_set_margin_bottom(brandRow, 8);
    GtkWidget* lblSwift = gtk_label_new("Swift");
    setCssClass(lblSwift, "swift-brand-swift", true);
    GtkWidget* lblNote = gtk_label_new("note");
    setCssClass(lblNote, "swift-brand-note", true);
    gtk_box_pack_start(GTK_BOX(brandRow), lblSwift, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(brandRow), lblNote, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(sidebar), brandRow, FALSE, FALSE, 0);

    auto addFilterBtn = [&](const char* icon, const char* text, int kFilter) {
        GtkWidget* btn = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
        setCssClass(btn, "swift-menu-row", true);
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        gtk_box_pack_start(GTK_BOX(row), gtk_image_new_from_icon_name(icon, GTK_ICON_SIZE_MENU), FALSE, FALSE, 0);
        GtkWidget* l = gtk_label_new(text);
        gtk_label_set_xalign(GTK_LABEL(l), 0.0f);
        gtk_box_pack_start(GTK_BOX(row), l, TRUE, TRUE, 0);
        gtk_container_add(GTK_CONTAINER(btn), row);
        g_object_set_data(G_OBJECT(btn), "kind-filter", GINT_TO_POINTER(kFilter));
        g_signal_connect(btn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                             self->activeFolderFilter.clear();
                             self->kindFilter = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(b), "kind-filter"));
                             self->refresh();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(sidebar), btn, FALSE, FALSE, 0);
    };

    addFilterBtn("folder-documents-symbolic", "Tất cả ghi chú", -1);
    addFilterBtn("xopp-document-new", "Sổ tay (.xopp)", 0);
    addFilterBtn("xopp-document-export-pdf", "Tài liệu PDF", 2);

    GtkWidget* secLbl = gtk_label_new("THƯ MỤC");
    setCssClass(secLbl, "swift-card-title", true);
    gtk_label_set_xalign(GTK_LABEL(secLbl), 0.0f);
    gtk_widget_set_margin_start(secLbl, 8);
    gtk_widget_set_margin_top(secLbl, 8);
    gtk_box_pack_start(GTK_BOX(sidebar), secLbl, FALSE, FALSE, 0);

    GtkWidget* foldersScroll = gtk_scrolled_window_new(nullptr, nullptr);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(foldersScroll), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    this->foldersBox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_container_add(GTK_CONTAINER(foldersScroll), this->foldersBox);
    gtk_box_pack_start(GTK_BOX(sidebar), foldersScroll, TRUE, TRUE, 0);

    gtk_box_pack_start(GTK_BOX(sidebar), gtk_separator_new(GTK_ORIENTATION_HORIZONTAL), FALSE, FALSE, 4);

    auto addBottomBtn = [&](const char* icon, const char* text, GCallback cb) {
        GtkWidget* btn = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
        setCssClass(btn, "swift-menu-row", true);
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        gtk_box_pack_start(GTK_BOX(row), gtk_image_new_from_icon_name(icon, GTK_ICON_SIZE_MENU), FALSE, FALSE, 0);
        GtkWidget* l = gtk_label_new(text);
        gtk_label_set_xalign(GTK_LABEL(l), 0.0f);
        gtk_box_pack_start(GTK_BOX(row), l, TRUE, TRUE, 0);
        gtk_container_add(GTK_CONTAINER(btn), row);
        g_signal_connect(btn, "clicked", cb, this);
        gtk_box_pack_start(GTK_BOX(sidebar), btn, FALSE, FALSE, 0);
    };

    addBottomBtn("folder-open-symbolic", "Đổi thư mục thư viện...", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                     static_cast<SwiftLibraryView*>(selfPtr)->chooseLibraryFolder();
                 }));
    addBottomBtn("xopp-document-open", "Mở tệp bất kỳ...", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                     auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                     self->shell->showEditor();
                     self->control->askToOpenFile();
                 }));
    addBottomBtn("go-previous-symbolic", "Quay lại trang viết", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                     static_cast<SwiftLibraryView*>(selfPtr)->shell->showEditor();
                 }));

    gtk_box_pack_start(GTK_BOX(this->rootWidget), sidebar, FALSE, FALSE, 0);

    // === MAIN RIGHT CONTENT AREA ===
    GtkWidget* mainCol = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);

    // Top Header Bar
    GtkWidget* header = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    gtk_widget_set_name(header, "swiftLibraryHeader");

    this->searchEntry = gtk_search_entry_new();
    gtk_entry_set_placeholder_text(GTK_ENTRY(this->searchEntry), "Tìm kiếm ghi chú (hỗ trợ tiếng Việt không dấu)...");
    gtk_widget_set_size_request(this->searchEntry, 320, -1);
    g_signal_connect(this->searchEntry, "search-changed", G_CALLBACK(+[](GtkSearchEntry* e, gpointer selfPtr) {
                         auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                         const char* txt = gtk_entry_get_text(GTK_ENTRY(e));
                         self->searchQuery = txt ? txt : "";
                         self->refresh();
                     }),
                     this);
    gtk_box_pack_start(GTK_BOX(header), this->searchEntry, FALSE, FALSE, 0);

    this->statusLabel = gtk_label_new("");
    setCssClass(this->statusLabel, "swift-hint", true);
    gtk_label_set_xalign(GTK_LABEL(this->statusLabel), 0.0f);
    gtk_box_pack_start(GTK_BOX(header), this->statusLabel, TRUE, TRUE, 4);

    GtkWidget* btnRefresh = gtk_button_new_from_icon_name("view-refresh-symbolic", GTK_ICON_SIZE_BUTTON);
    gtk_button_set_relief(GTK_BUTTON(btnRefresh), GTK_RELIEF_NONE);
    gtk_widget_set_tooltip_text(btnRefresh, "Làm mới thư viện");
    g_signal_connect(btnRefresh, "clicked", G_CALLBACK(+[](GtkButton*, gpointer selfPtr) {
                         static_cast<SwiftLibraryView*>(selfPtr)->refresh();
                     }),
                     this);
    gtk_box_pack_end(GTK_BOX(header), btnRefresh, FALSE, FALSE, 0);

    GtkWidget* btnNewNote = gtk_button_new_with_label("+ Ghi chú mới");
    setCssClass(btnNewNote, "swift-primary-btn", true);
    g_signal_connect(btnNewNote, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         static_cast<SwiftLibraryView*>(selfPtr)->openNewNotePopover(GTK_WIDGET(b));
                     }),
                     this);
    gtk_box_pack_end(GTK_BOX(header), btnNewNote, FALSE, FALSE, 0);

    gtk_box_pack_start(GTK_BOX(mainCol), header, FALSE, FALSE, 0);

    // Note Cards FlowBox inside ScrolledWindow
    GtkWidget* scroll = gtk_scrolled_window_new(nullptr, nullptr);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scroll), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);

    this->flowBox = gtk_flow_box_new();
    gtk_widget_set_valign(this->flowBox, GTK_ALIGN_START);
    gtk_flow_box_set_selection_mode(GTK_FLOW_BOX(this->flowBox), GTK_SELECTION_NONE);
    gtk_flow_box_set_homogeneous(GTK_FLOW_BOX(this->flowBox), TRUE);
    gtk_flow_box_set_max_children_per_line(GTK_FLOW_BOX(this->flowBox), 6);
    gtk_flow_box_set_column_spacing(GTK_FLOW_BOX(this->flowBox), 14);
    gtk_flow_box_set_row_spacing(GTK_FLOW_BOX(this->flowBox), 14);
    gtk_widget_set_margin_top(this->flowBox, 18);
    gtk_widget_set_margin_bottom(this->flowBox, 18);
    gtk_widget_set_margin_start(this->flowBox, 18);
    gtk_widget_set_margin_end(this->flowBox, 18);

    gtk_container_add(GTK_CONTAINER(scroll), this->flowBox);
    gtk_box_pack_start(GTK_BOX(mainCol), scroll, TRUE, TRUE, 0);

    gtk_box_pack_start(GTK_BOX(this->rootWidget), mainCol, TRUE, TRUE, 0);
    gtk_widget_show_all(this->rootWidget);
}

void SwiftLibraryView::refresh() {
    const std::string& libDir = this->shell->getSettings().libraryDir;
    if (!libDir.empty()) {
        std::error_code ec;
        std::filesystem::create_directories(libDir, ec);
    }

    auto allNotes = SwiftBridge::scanLibrary(libDir, "");
    rebuildSidebarFolders(allNotes);

    auto scanned = this->searchQuery.empty() ? allNotes : SwiftBridge::scanLibrary(libDir, this->searchQuery);
    std::vector<NoteItem> filtered;
    filtered.reserve(scanned.size());
    for (const auto& item: scanned) {
        if (!this->activeFolderFilter.empty() && item.folder != this->activeFolderFilter) {
            continue;
        }
        if (this->kindFilter == 0 && item.kind == SN_NOTE_KIND_PDF) {
            continue;
        }
        if (this->kindFilter == 2 && item.kind != SN_NOTE_KIND_PDF) {
            continue;
        }
        filtered.push_back(item);
    }

    char statusBuf[256];
    std::snprintf(statusBuf, sizeof(statusBuf), "%zu ghi chú  •  %s", filtered.size(), libDir.c_str());
    gtk_label_set_text(GTK_LABEL(this->statusLabel), statusBuf);

    rebuildCards(filtered);
}

void SwiftLibraryView::rebuildSidebarFolders(const std::vector<NoteItem>& allNotes) {
    GList* children = gtk_container_get_children(GTK_CONTAINER(this->foldersBox));
    for (GList* it = children; it != nullptr; it = it->next) {
        gtk_widget_destroy(GTK_WIDGET(it->data));
    }
    g_list_free(children);

    std::set<std::string> uniqueFolders;
    for (const auto& n: allNotes) {
        if (!n.folder.empty()) {
            uniqueFolders.insert(n.folder);
        }
    }

    for (const auto& f: uniqueFolders) {
        GtkWidget* btn = gtk_button_new();
        gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
        setCssClass(btn, "swift-menu-row", true);
        if (this->activeFolderFilter == f) {
            setCssClass(btn, "swift-active", true);
        }
        GtkWidget* row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        gtk_box_pack_start(GTK_BOX(row), gtk_image_new_from_icon_name("folder-symbolic", GTK_ICON_SIZE_MENU), FALSE,
                           FALSE, 0);
        GtkWidget* l = gtk_label_new(f.c_str());
        gtk_label_set_xalign(GTK_LABEL(l), 0.0f);
        gtk_label_set_ellipsize(GTK_LABEL(l), PANGO_ELLIPSIZE_END);
        gtk_box_pack_start(GTK_BOX(row), l, TRUE, TRUE, 0);
        gtk_container_add(GTK_CONTAINER(btn), row);

        g_object_set_data_full(G_OBJECT(btn), "folder-name", g_strdup(f.c_str()), g_free);
        g_signal_connect(btn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                             const char* fn = static_cast<const char*>(g_object_get_data(G_OBJECT(b), "folder-name"));
                             self->activeFolderFilter = fn ? fn : "";
                             self->kindFilter = -1;
                             self->refresh();
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(this->foldersBox), btn, FALSE, FALSE, 0);
    }
    gtk_widget_show_all(this->foldersBox);
}

GtkWidget* SwiftLibraryView::makeNoteCard(const NoteItem& item) {
    GtkWidget* btn = gtk_button_new();
    gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
    setCssClass(btn, "swift-note-card", true);
    gtk_widget_set_size_request(btn, 184, 224);

    GtkWidget* box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);

    // Thumbnail preview area
    GtkWidget* thumbFrame = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_widget_set_size_request(thumbFrame, 160, 140);
    gtk_widget_set_halign(thumbFrame, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(thumbFrame, GTK_ALIGN_CENTER);

    GdkPixbuf* thumb = nullptr;
    if (item.hasPreview && item.kind != SN_NOTE_KIND_PDF) {
        if (auto png = SwiftBridge::extractXoppPreview(item.path)) {
            thumb = decodePngThumbnail(*png, 158, 138);
        }
    }

    if (thumb) {
        GtkWidget* img = gtk_image_new_from_pixbuf(thumb);
        g_object_unref(thumb);
        gtk_box_pack_start(GTK_BOX(thumbFrame), img, TRUE, TRUE, 0);
    } else {
        const char* icon = (item.kind == SN_NOTE_KIND_PDF) ? "xopp-document-export-pdf" : "xopp-sidebar-page-preview";
        GtkWidget* img = gtk_image_new_from_icon_name(icon, GTK_ICON_SIZE_DIALOG);
        gtk_box_pack_start(GTK_BOX(thumbFrame), img, TRUE, TRUE, 0);
    }
    gtk_box_pack_start(GTK_BOX(box), thumbFrame, TRUE, TRUE, 0);

    // Title
    GtkWidget* titleLbl = gtk_label_new(item.title.c_str());
    gtk_label_set_xalign(GTK_LABEL(titleLbl), 0.0f);
    gtk_label_set_ellipsize(GTK_LABEL(titleLbl), PANGO_ELLIPSIZE_END);
    gtk_label_set_max_width_chars(GTK_LABEL(titleLbl), 22);
    gtk_box_pack_start(GTK_BOX(box), titleLbl, FALSE, FALSE, 0);

    // Subtitle: pages + modified date
    std::string sub;
    if (item.kind == SN_NOTE_KIND_PDF) {
        sub = "PDF";
    } else if (item.pageCount > 0) {
        sub = std::to_string(item.pageCount) + " trang";
    } else {
        sub = "Sổ tay";
    }
    const std::string dt = formatShortDate(item.modifiedEpochSecs);
    if (!dt.empty()) {
        sub += " • " + dt;
    }
    GtkWidget* subLbl = gtk_label_new(sub.c_str());
    setCssClass(subLbl, "swift-hint", true);
    gtk_label_set_xalign(GTK_LABEL(subLbl), 0.0f);
    gtk_label_set_ellipsize(GTK_LABEL(subLbl), PANGO_ELLIPSIZE_END);
    gtk_box_pack_start(GTK_BOX(box), subLbl, FALSE, FALSE, 0);

    gtk_container_add(GTK_CONTAINER(btn), box);

    g_object_set_data_full(G_OBJECT(btn), "note-path", g_strdup(item.path.c_str()), g_free);
    g_signal_connect(btn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                         auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                         const char* p = static_cast<const char*>(g_object_get_data(G_OBJECT(b), "note-path"));
                         if (p && p[0] != '\0') {
                             self->shell->showEditor();
                             self->control->openFile(fs::path(reinterpret_cast<const char8_t*>(p)));
                         }
                     }),
                     this);

    return btn;
}

void SwiftLibraryView::rebuildCards(const std::vector<NoteItem>& items) {
    GList* children = gtk_container_get_children(GTK_CONTAINER(this->flowBox));
    for (GList* it = children; it != nullptr; it = it->next) {
        gtk_widget_destroy(GTK_WIDGET(it->data));
    }
    g_list_free(children);

    if (items.empty()) {
        GtkWidget* emptyBox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
        gtk_widget_set_margin_top(emptyBox, 48);
        GtkWidget* lbl = gtk_label_new(
                this->searchQuery.empty()
                        ? "Chưa có ghi chú nào trong thư mục Thư viện.\nBấm \"+ Ghi chú mới\" để bắt đầu viết ngay!"
                        : "Không tìm thấy ghi chú phù hợp với từ khóa.");
        gtk_label_set_justify(GTK_LABEL(lbl), GTK_JUSTIFY_CENTER);
        setCssClass(lbl, "swift-hint", true);
        gtk_box_pack_start(GTK_BOX(emptyBox), lbl, FALSE, FALSE, 0);
        gtk_flow_box_insert(GTK_FLOW_BOX(this->flowBox), emptyBox, -1);
    } else {
        for (const auto& item: items) {
            gtk_flow_box_insert(GTK_FLOW_BOX(this->flowBox), makeNoteCard(item), -1);
        }
    }
    gtk_widget_show_all(this->flowBox);
}

void SwiftLibraryView::openNewNotePopover(GtkWidget* anchor) {
    if (this->activePopover) {
        GtkWidget* oldPop = this->activePopover;
        this->activePopover = nullptr;
        gtk_popover_popdown(GTK_POPOVER(oldPop));
    }
    GtkWidget* pop = gtk_popover_new(anchor);
    gtk_popover_set_position(GTK_POPOVER(pop), GTK_POS_BOTTOM);
    setCssClass(pop, "swift-popover", true);
    if (this->control->getWindow() && this->control->getWindow()->isDarkTheme()) {
        setCssClass(pop, "darkMode", true);
    }
    g_signal_connect(pop, "closed", G_CALLBACK(+[](GtkPopover* p, gpointer selfPtr) {
                         auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                         GtkWidget* w = GTK_WIDGET(p);
                         if (self->activePopover == w) {
                             self->activePopover = nullptr;
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

    GtkWidget* box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    gtk_widget_set_size_request(box, 230, -1);

    GtkWidget* title = gtk_label_new("Chọn mẫu giấy cho ghi chú mới");
    setCssClass(title, "swift-section-title", true);
    gtk_label_set_xalign(GTK_LABEL(title), 0.0f);
    gtk_box_pack_start(GTK_BOX(box), title, FALSE, FALSE, 0);

    const std::array<std::pair<const char*, const char*>, 7> templates = {{
            {"Kẻ ngang (Ruled)", "ruled"},
            {"Giấy trắng trơn (Plain)", "plain"},
            {"Ô ly vuông (Graph)", "graph"},
            {"Chấm bi (Dotted)", "dotted"},
            {"Cornell Notes", "cornell"},
            {"Kẻ ngang có lề (Lined)", "lined"},
            {"Bảng đen (Blackboard)", "blackboard"},
    }};
    for (const auto& [label, id]: templates) {
        GtkWidget* btn = gtk_button_new_with_label(label);
        gtk_button_set_relief(GTK_BUTTON(btn), GTK_RELIEF_NONE);
        setCssClass(btn, "swift-menu-row", true);
        g_object_set_data_full(G_OBJECT(btn), "tpl-id", g_strdup(id), g_free);
        g_signal_connect(btn, "clicked", G_CALLBACK(+[](GtkButton* b, gpointer selfPtr) {
                             auto* self = static_cast<SwiftLibraryView*>(selfPtr);
                             const char* tpl = static_cast<const char*>(g_object_get_data(G_OBJECT(b), "tpl-id"));
                             const std::string tplCopy = tpl ? tpl : "";
                             if (self->activePopover) {
                                 GtkWidget* curPop = self->activePopover;
                                 self->activePopover = nullptr;
                                 gtk_popover_popdown(GTK_POPOVER(curPop));
                             }
                             if (!tplCopy.empty()) {
                                 self->createNoteWithTemplate(tplCopy);
                             }
                         }),
                         this);
        gtk_box_pack_start(GTK_BOX(box), btn, FALSE, FALSE, 0);
    }

    gtk_container_add(GTK_CONTAINER(pop), box);
    gtk_widget_show_all(box);
    gtk_popover_popup(GTK_POPOVER(pop));
}

void SwiftLibraryView::createNoteWithTemplate(const std::string& templateId) {
    this->shell->showEditor();
    this->control->newFile();
    this->shell->applyPaperTemplate(templateId, /*allPages=*/false);
}

void SwiftLibraryView::chooseLibraryFolder() {
    GtkWidget* dlg = gtk_file_chooser_dialog_new("Chọn thư mục Thư viện Swift note", this->control->getGtkWindow(),
                                                 GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER, "Hủy", GTK_RESPONSE_CANCEL,
                                                 "Chọn thư mục", GTK_RESPONSE_ACCEPT, nullptr);
    const std::string& curDir = this->shell->getSettings().libraryDir;
    if (!curDir.empty()) {
        gtk_file_chooser_set_filename(GTK_FILE_CHOOSER(dlg), curDir.c_str());
    }
    if (gtk_dialog_run(GTK_DIALOG(dlg)) == GTK_RESPONSE_ACCEPT) {
        gchar* folder = gtk_file_chooser_get_filename(GTK_FILE_CHOOSER(dlg));
        if (folder) {
            this->shell->getSettingsMut().libraryDir = folder;
            SwiftBridge::setSettings(this->shell->getSettings());
            SwiftBridge::saveState();
            g_free(folder);
            this->activeFolderFilter.clear();
            refresh();
        }
    }
    gtk_widget_destroy(dlg);
}

}  // namespace swiftnote

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum SnStatus {
    SN_OK = 0,
    SN_ERR_NULL_POINTER = 1,
    SN_ERR_INVALID_UTF8 = 2,
    SN_ERR_IO = 3,
    SN_ERR_PARSE = 4,
    SN_ERR_MEDIA = 5,
    SN_ERR_OUT_OF_RANGE = 6,
    SN_ERR_PANIC = 99
} SnStatus;

typedef enum SnThemeMode {
    SN_THEME_LIGHT = 0,
    SN_THEME_DARK = 1,
    SN_THEME_SYSTEM = 2
} SnThemeMode;

typedef enum SnUiLayout {
    SN_LAYOUT_PURE_TOP_BAR = 0,
    SN_LAYOUT_CANVAS_PILL = 1,
    SN_LAYOUT_ZEN_BOTTOM_DOCK = 2
} SnUiLayout;

typedef struct SnShortcutEntry {
    char action_id[48];
    char label[96];
    char key[24];
} SnShortcutEntry;

typedef enum SnPenKind {
    SN_PEN_KIND_PEN = 0,
    SN_PEN_KIND_HIGHLIGHTER = 1
} SnPenKind;

typedef enum SnLineStyle {
    SN_LINE_STYLE_SOLID = 0,
    SN_LINE_STYLE_DASHED = 1,
    SN_LINE_STYLE_DOTTED = 2
} SnLineStyle;

typedef enum SnNoteKind {
    SN_NOTE_KIND_XOPP = 0,
    SN_NOTE_KIND_XOJ = 1,
    SN_NOTE_KIND_PDF = 2
} SnNoteKind;

typedef struct SnByteBuffer {
    uint8_t* data;
    size_t len;
    size_t cap;
} SnByteBuffer;

typedef struct SnPenSlot {
    uint32_t id;
    char label[64];
    SnPenKind kind;
    uint32_t color_rgb;
    double width_pt;
    bool pressure_sensitive;
    SnLineStyle line_style;
    bool ink_to_shape;
} SnPenSlot;

typedef struct SnAppSettings {
    SnThemeMode theme;
    SnUiLayout ui_layout;
    uint32_t active_pen_slot;
    uint32_t pen_slot_count;
    bool eraser_auto_return;
    bool blackboard_enabled;
    bool blackboard_auto_invert_bw_ink;
    uint32_t blackboard_surface_rgb;
    double close_up_zoom_factor;
    bool close_up_auto_advance;
    uint32_t close_up_panel_height_px;
    double close_up_margin_trigger_ratio;
    char library_dir[512];
    char default_paper_template[64];
} SnAppSettings;

typedef struct SnNoteItem {
    char title[256];
    char path[512];
    char folder[256];
    SnNoteKind kind;
    uint64_t modified_epoch_secs;
    uint64_t size_bytes;
    uint32_t page_count;
    bool has_preview;
} SnNoteItem;

typedef struct SnNoteList {
    SnNoteItem* items;
    size_t len;
    size_t cap;
} SnNoteList;

/* Memory management */
void sn_string_free(char* ptr);
void sn_buffer_free(SnByteBuffer buf);
void sn_note_list_free(SnNoteList list);

/* Library version */
const char* sn_version(void);

/* Config (typed C ABI backed by atomic JSON persistence in Rust) */
SnStatus sn_config_default_path(char** out_path);
SnStatus sn_config_load_json(const char* path_utf8, char** out_json);
SnStatus sn_config_save_json(const char* path_utf8, const char* json_utf8);

SnStatus sn_state_init(const char* path_utf8);
SnStatus sn_state_get_settings(SnAppSettings* out_settings);
SnStatus sn_state_set_settings(const SnAppSettings* settings);
uint32_t sn_state_pen_slot_count(void);
SnStatus sn_state_get_pen_slot(uint32_t index, SnPenSlot* out_slot);
SnStatus sn_state_set_pen_slot(uint32_t index, const SnPenSlot* slot);
SnStatus sn_state_add_pen_slot(const SnPenSlot* slot, uint32_t* out_index);
SnStatus sn_state_remove_pen_slot(uint32_t index);
uint32_t sn_state_shortcut_count(void);
SnStatus sn_state_get_shortcut(uint32_t index, SnShortcutEntry* out_entry);
SnStatus sn_state_set_shortcut_key(const char* action_id_utf8, const char* key_utf8);
SnStatus sn_state_reset_shortcuts(void);
SnStatus sn_state_save(void);

/* Blackboard Auto Ink Inversion (O(1), non-destructive display-layer RGB transform) */
uint32_t sn_invert_bw_ink_rgb(uint32_t rgb24);

/* Lossless Clipboard Media Normalization */
SnStatus sn_normalize_clipboard_png(const uint8_t* png_bytes,
                                    size_t png_len,
                                    SnByteBuffer* out_png,
                                    uint32_t* out_width,
                                    uint32_t* out_height);

SnStatus sn_pixels_to_lossless_png(const uint8_t* pixels,
                                   size_t pixels_len,
                                   uint32_t width,
                                   uint32_t height,
                                   uint32_t stride,
                                   bool is_bgra,
                                   bool fix_all_zero_alpha,
                                   SnByteBuffer* out_png);

/* Note Library Indexer & Vietnamese Diacritic-Insensitive Search */
SnStatus sn_library_scan_json(const char* root_dir_utf8,
                              const char* query_utf8,
                              char** out_json);

SnStatus sn_library_scan_list(const char* root_dir_utf8,
                              const char* query_utf8,
                              SnNoteList* out_list);

SnStatus sn_library_extract_xopp_preview(const char* file_path_utf8,
                                         SnByteBuffer* out_png);

#ifdef __cplusplus
}
#endif

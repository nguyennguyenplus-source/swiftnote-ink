use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use base64::Engine;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::{Result, SwiftNoteError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteFileKind {
    Xopp,
    Xoj,
    Pdf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEntry {
    pub title: String,
    pub path: String,
    pub folder: String,
    pub kind: NoteFileKind,
    pub modified_epoch_secs: u64,
    pub size_bytes: u64,
    pub page_count: u32,
    pub has_preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryScanResult {
    pub root_dir: String,
    pub folders: Vec<String>,
    pub notes: Vec<NoteEntry>,
}

/// Scans `root_dir` for `.xopp`, `.xoj`, and `.pdf` notes, extracting fast metadata
/// and optionally filtering by a Vietnamese diacritic-insensitive query.
pub fn scan_library(root_dir: &Path, query: &str) -> Result<LibraryScanResult> {
    if !root_dir.exists() {
        return Ok(LibraryScanResult {
            root_dir: root_dir.to_string_lossy().into_owned(),
            folders: Vec::new(),
            notes: Vec::new(),
        });
    }

    let normalized_query = fold_vietnamese(query.trim());
    let mut folders = Vec::new();
    let mut notes = Vec::new();

    for entry in WalkDir::new(root_dir)
        .follow_links(false)
        .max_depth(6)
        .into_iter()
        .flatten()
    {
        let path = entry.path();
        if path == root_dir {
            continue;
        }
        if is_hidden(path) {
            continue;
        }

        if entry.file_type().is_dir() {
            if let Ok(rel) = path.strip_prefix(root_dir) {
                folders.push(rel.to_string_lossy().replace('\\', "/"));
            }
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        let Some(kind) = classify_note(path) else {
            continue;
        };

        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());

        let folder = path
            .parent()
            .and_then(|p| p.strip_prefix(root_dir).ok())
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        if !normalized_query.is_empty() {
            let haystack = format!("{} {}", fold_vietnamese(&title), fold_vietnamese(&folder));
            if !haystack.contains(&normalized_query) {
                continue;
            }
        }

        let meta = fs::metadata(path).ok();
        let size_bytes = meta.as_ref().map_or(0, fs::Metadata::len);
        let modified_epoch_secs = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());

        let (page_count, has_preview) = match kind {
            NoteFileKind::Xopp | NoteFileKind::Xoj => peek_xopp_summary(path),
            NoteFileKind::Pdf => (0, false),
        };

        notes.push(NoteEntry {
            title,
            path: path.to_string_lossy().into_owned(),
            folder,
            kind,
            modified_epoch_secs,
            size_bytes,
            page_count,
            has_preview,
        });
    }

    folders.sort();
    notes.sort_by(|a, b| {
        b.modified_epoch_secs
            .cmp(&a.modified_epoch_secs)
            .then_with(|| a.title.cmp(&b.title))
    });

    Ok(LibraryScanResult {
        root_dir: root_dir.to_string_lossy().into_owned(),
        folders,
        notes,
    })
}

/// Extracts the embedded base64 `<preview>...</preview>` PNG thumbnail from a `.xopp`/`.xoj` file.
pub fn extract_xopp_preview(path: &Path) -> Result<Vec<u8>> {
    let xml = read_xopp_xml_prefix(path, 512 * 1024)?;
    let start_tag = "<preview>";
    let end_tag = "</preview>";

    let start_idx = xml
        .find(start_tag)
        .ok_or_else(|| SwiftNoteError::Media("no preview tag in .xopp".to_string()))?
        + start_tag.len();
    let end_idx = xml[start_idx..]
        .find(end_tag)
        .ok_or_else(|| SwiftNoteError::Media("unterminated preview tag".to_string()))?
        + start_idx;

    let b64: String = xml[start_idx..end_idx]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| SwiftNoteError::Media(format!("invalid base64 preview: {e}")))
}

fn classify_note(path: &Path) -> Option<NoteFileKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "xopp" => Some(NoteFileKind::Xopp),
        "xoj" => Some(NoteFileKind::Xoj),
        "pdf" => Some(NoteFileKind::Pdf),
        _ => None,
    }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn peek_xopp_summary(path: &Path) -> (u32, bool) {
    // Limit decompressed scan to 4 MiB to stay ultra-fast even on huge notebooks
    let Ok(xml) = read_xopp_xml_prefix(path, 4 * 1024 * 1024) else {
        return (0, false);
    };
    let has_preview = xml.contains("<preview>");
    let page_count = xml.matches("<page ").count() as u32;
    (page_count, has_preview)
}

fn read_xopp_xml_prefix(path: &Path, max_bytes: usize) -> Result<String> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 2];
    let n = file.read(&mut header)?;
    if n == 0 {
        return Ok(String::new());
    }

    let file = File::open(path)?;
    let mut buf = Vec::new();
    if n == 2 && header == [0x1F, 0x8B] {
        // Standard Gzip-compressed .xopp/.xoj
        let gz = GzDecoder::new(file);
        gz.take(max_bytes as u64).read_to_end(&mut buf)?;
    } else {
        // Plain XML fallback
        file.take(max_bytes as u64).read_to_end(&mut buf)?;
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Folds Vietnamese diacritics and lowercases text for fast, forgiving notebook search.
/// Example: `"Toán 12 - Đạo hàm"` -> `"toan 12 - dao ham"`.
pub fn fold_vietnamese(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let mapped = match ch {
            'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ' | 'ắ'
            | 'ặ' | 'ẳ' | 'ẵ' | 'À' | 'Á' | 'Ạ' | 'Ả' | 'Ã' | 'Â' | 'Ầ' | 'Ấ' | 'Ậ' | 'Ẩ' | 'Ẫ'
            | 'Ă' | 'Ằ' | 'Ắ' | 'Ặ' | 'Ẳ' | 'Ẵ' => 'a',
            'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' | 'È' | 'É' | 'Ẹ'
            | 'Ẻ' | 'Ẽ' | 'Ê' | 'Ề' | 'Ế' | 'Ệ' | 'Ể' | 'Ễ' => 'e',
            'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' | 'Ì' | 'Í' | 'Ị' | 'Ỉ' | 'Ĩ' => 'i',
            'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ' | 'ớ'
            | 'ợ' | 'ở' | 'ỡ' | 'Ò' | 'Ó' | 'Ọ' | 'Ỏ' | 'Õ' | 'Ô' | 'Ồ' | 'Ố' | 'Ộ' | 'Ổ' | 'Ỗ'
            | 'Ơ' | 'Ờ' | 'Ớ' | 'Ợ' | 'Ở' | 'Ỡ' => 'o',
            'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' | 'Ù' | 'Ú' | 'Ụ'
            | 'Ủ' | 'Ũ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ự' | 'Ử' | 'Ữ' => 'u',
            'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' | 'Ỳ' | 'Ý' | 'Ỵ' | 'Ỷ' | 'Ỹ' => 'y',
            'đ' | 'Đ' => 'd',
            other => {
                for lower in other.to_lowercase() {
                    out.push(lower);
                }
                continue;
            }
        };
        out.push(mapped);
    }
    out
}

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::error::CoreError;
use crate::storage::{load_swnote, save_swnote_atomic};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub note_id: String,
    pub title: String,
    pub updated_unix_ms: u64,
    pub page_count: usize,
    pub file_name: String,
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryIndex {
    pub schema_version: u32,
    pub entries: Vec<LibraryEntry>,
}

#[derive(Debug, Clone)]
pub struct NoteLibrary {
    root: PathBuf,
    index: LibraryIndex,
}

impl NoteLibrary {
    pub fn open_default() -> Result<Self, CoreError> {
        let dirs = ProjectDirs::from("io", "swiftnote", "SwiftNote").ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot resolve OS data directory",
            ))
        })?;
        Self::open_at(dirs.data_dir().join("library"))
    }

    pub fn open_at(root: PathBuf) -> Result<Self, CoreError> {
        fs::create_dir_all(root.join("notes"))?;
        let index_path = root.join("library.json");
        let index = if index_path.exists() {
            let bytes = fs::read(&index_path)?;
            serde_json::from_slice(&bytes)?
        } else {
            LibraryIndex {
                schema_version: 1,
                entries: Vec::new(),
            }
        };
        Ok(Self { root, index })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list_recent_notes(&self) -> Vec<LibraryEntry> {
        let mut list = self.index.entries.clone();
        list.sort_by_key(|a| std::cmp::Reverse(a.updated_unix_ms));
        list
    }

    pub fn create_note(&mut self, title: impl Into<String>) -> Result<Document, CoreError> {
        let now = now_unix_ms();
        let note_id = format!("note_{now}_{}", self.index.entries.len() + 1);
        let doc = Document::new_untitled(note_id, title, now);
        self.save_note(&doc)?;
        Ok(doc)
    }

    pub fn save_note(&mut self, doc: &Document) -> Result<(), CoreError> {
        let file_name = format!("{}.swnote", doc.note_id);
        let path = self.root.join("notes").join(&file_name);
        save_swnote_atomic(doc, &path)?;

        let entry = LibraryEntry {
            note_id: doc.note_id.clone(),
            title: doc.title.clone(),
            updated_unix_ms: doc.updated_unix_ms.max(now_unix_ms()),
            page_count: doc.pages.len(),
            file_name,
            folder_id: None,
        };
        if let Some(existing) = self
            .index
            .entries
            .iter_mut()
            .find(|e| e.note_id == doc.note_id)
        {
            *existing = entry;
        } else {
            self.index.entries.push(entry);
        }
        self.persist_index()
    }

    pub fn open_note(&self, note_id: &str) -> Result<Document, CoreError> {
        let entry = self
            .index
            .entries
            .iter()
            .find(|e| e.note_id == note_id)
            .ok_or_else(|| CoreError::NoteNotFound(note_id.to_string()))?;
        let path = self.root.join("notes").join(&entry.file_name);
        load_swnote(&path)
    }

    pub fn rename_note(
        &mut self,
        note_id: &str,
        new_title: impl Into<String>,
    ) -> Result<(), CoreError> {
        let new_title = new_title.into();
        let mut doc = self.open_note(note_id)?;
        doc.title = new_title;
        doc.updated_unix_ms = now_unix_ms();
        self.save_note(&doc)
    }

    pub fn delete_note(&mut self, note_id: &str) -> Result<(), CoreError> {
        let pos = self
            .index
            .entries
            .iter()
            .position(|e| e.note_id == note_id)
            .ok_or_else(|| CoreError::NoteNotFound(note_id.to_string()))?;
        let removed = self.index.entries.remove(pos);
        let path = self.root.join("notes").join(&removed.file_name);
        if path.exists() {
            fs::remove_file(path)?;
        }
        self.persist_index()
    }

    pub fn import_swnote(&mut self, external_path: &Path) -> Result<Document, CoreError> {
        let mut doc = load_swnote(external_path)?;
        let now = now_unix_ms();
        if self.index.entries.iter().any(|e| e.note_id == doc.note_id) {
            doc.note_id = format!("note_{now}_{}", self.index.entries.len() + 1);
        }
        doc.updated_unix_ms = now;
        self.save_note(&doc)?;
        Ok(doc)
    }

    pub fn export_swnote_copy(&self, doc: &Document, dest_path: &Path) -> Result<(), CoreError> {
        save_swnote_atomic(doc, dest_path)
    }

    fn persist_index(&self) -> Result<(), CoreError> {
        let index_path = self.root.join("library.json");
        let tmp_path = self
            .root
            .join(format!(".library.{}.json.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(&self.index)?;
        {
            let mut f = File::create(&tmp_path)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp_path, &index_path)?;
        Ok(())
    }
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

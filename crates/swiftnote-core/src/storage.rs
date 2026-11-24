use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::document::{AssetId, Document, ImageAsset, ImagePlacement, Page, PageId, PdfAssetMeta};
use crate::error::{CoreError, MAX_ARCHIVE_UNCOMPRESSED_BYTES};
use crate::paper::{PageBackground, PaperStandard};
use crate::stroke::Stroke;

const SWNOTE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct ManifestV1 {
    schema_version: u32,
    note_id: String,
    title: String,
    created_unix_ms: u64,
    updated_unix_ms: u64,
    default_standard: PaperStandard,
    default_background: PageBackground,
    pages_meta: Vec<PageMetaV1>,
    pdfs_meta: Vec<PdfMetaV1>,
    next_stroke_id: u64,
    next_page_id: u64,
    next_image_id: u64,
    page_gap_pt: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PageMetaV1 {
    id: PageId,
    width_pt: f64,
    height_pt: f64,
    background: PageBackground,
}

#[derive(Debug, Serialize, Deserialize)]
struct PdfMetaV1 {
    id: AssetId,
    encrypted: bool,
    page_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct PageContentV1 {
    strokes: Vec<Stroke>,
    images: Vec<ImagePlacement>,
}

pub fn save_swnote_atomic(doc: &Document, path: &Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("note.swnote");
    let tmp_name = format!(".{file_name}.{}.tmp", std::process::id());
    let tmp_path = path.with_file_name(tmp_name);

    let write_res = write_swnote_zip(doc, &tmp_path);
    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(CoreError::Io(err));
    }

    Ok(())
}

fn write_swnote_zip(doc: &Document, tmp_path: &Path) -> Result<(), CoreError> {
    let file = File::create(tmp_path)?;
    let mut writer = ZipWriter::new(file);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    let manifest = ManifestV1 {
        schema_version: SWNOTE_SCHEMA_VERSION,
        note_id: doc.note_id.clone(),
        title: doc.title.clone(),
        created_unix_ms: doc.created_unix_ms,
        updated_unix_ms: doc.updated_unix_ms,
        default_standard: doc.default_standard,
        default_background: doc.default_background.clone(),
        pages_meta: doc
            .pages
            .iter()
            .map(|p| PageMetaV1 {
                id: p.id,
                width_pt: p.width_pt,
                height_pt: p.height_pt,
                background: p.background.clone(),
            })
            .collect(),
        pdfs_meta: doc
            .pdfs
            .values()
            .map(|pdf| PdfMetaV1 {
                id: pdf.id,
                encrypted: pdf.encrypted,
                page_count: pdf.page_count,
            })
            .collect(),
        next_stroke_id: doc.next_stroke_id,
        next_page_id: doc.next_page_id,
        next_image_id: doc.next_image_id,
        page_gap_pt: doc.page_gap_pt,
    };

    writer.start_file("manifest.json", deflated)?;
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    writer.write_all(&manifest_bytes)?;

    for (idx, page) in doc.pages.iter().enumerate() {
        let entry_name = format!("pages/p_{idx:04}.json");
        writer.start_file(entry_name, deflated)?;
        let content = PageContentV1 {
            strokes: page.strokes.clone(),
            images: page.images.clone(),
        };
        let bytes = serde_json::to_vec(&content)?;
        writer.write_all(&bytes)?;
    }

    let mut written_assets = HashSet::new();
    for asset in doc.images.values() {
        if written_assets.insert(asset.id) {
            let entry_name = format!("assets/{}.png", asset.id.to_hex());
            writer.start_file(entry_name, stored)?;
            writer.write_all(&asset.png_bytes)?;
        }
    }
    for pdf in doc.pdfs.values() {
        if written_assets.insert(pdf.id) {
            let entry_name = format!("assets/{}.pdf", pdf.id.to_hex());
            writer.start_file(entry_name, stored)?;
            writer.write_all(&pdf.raw_bytes)?;
        }
    }

    let finished_file = writer.finish()?;
    finished_file.sync_all()?;
    Ok(())
}

pub fn load_swnote(path: &Path) -> Result<Document, CoreError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(BufReader::new(file))?;
    let mut total_read: u64 = 0;

    let manifest_bytes = read_entry_capped(&mut archive, "manifest.json", &mut total_read)?;
    let manifest: ManifestV1 = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != SWNOTE_SCHEMA_VERSION {
        return Err(CoreError::InvalidArchive(format!(
            "unsupported schema version {}",
            manifest.schema_version
        )));
    }

    let mut pages = Vec::with_capacity(manifest.pages_meta.len());
    let mut needed_images = HashSet::new();
    let mut needed_pdfs = HashSet::new();

    for (idx, meta) in manifest.pages_meta.iter().enumerate() {
        let entry_name = format!("pages/p_{idx:04}.json");
        let bytes = read_entry_capped(&mut archive, &entry_name, &mut total_read)?;
        let content: PageContentV1 = serde_json::from_slice(&bytes)?;
        for img in &content.images {
            needed_images.insert(img.asset_id);
        }
        if let PageBackground::Pdf { asset_id, .. } = &meta.background {
            needed_pdfs.insert(AssetId(*asset_id));
        } else if let PageBackground::CustomImage { asset_id } = &meta.background {
            needed_images.insert(AssetId(*asset_id));
        }

        let mut page = Page {
            id: meta.id,
            width_pt: meta.width_pt,
            height_pt: meta.height_pt,
            background: meta.background.clone(),
            strokes: content.strokes,
            images: content.images,
            index: Default::default(),
        };
        page.rebuild_index();
        pages.push(page);
    }

    let mut images = HashMap::new();
    for asset_id in needed_images {
        let entry_name = format!("assets/{}.png", asset_id.to_hex());
        let png_bytes = read_entry_capped(&mut archive, &entry_name, &mut total_read)?;
        let asset = ImageAsset::from_png_bytes(png_bytes)?;
        images.insert(asset.id, asset);
    }

    let mut pdf_meta_map: HashMap<AssetId, &PdfMetaV1> = HashMap::new();
    for m in &manifest.pdfs_meta {
        pdf_meta_map.insert(m.id, m);
    }

    let mut pdfs = HashMap::new();
    for asset_id in needed_pdfs {
        let entry_name = format!("assets/{}.pdf", asset_id.to_hex());
        let raw_bytes = read_entry_capped(&mut archive, &entry_name, &mut total_read)?;
        let meta_info = pdf_meta_map.get(&asset_id);
        let encrypted = meta_info.is_some_and(|m| m.encrypted);
        let page_count = meta_info.map_or(0, |m| m.page_count);
        pdfs.insert(
            asset_id,
            PdfAssetMeta {
                id: asset_id,
                encrypted,
                page_count,
                raw_bytes,
            },
        );
    }

    Ok(Document {
        note_id: manifest.note_id,
        title: manifest.title,
        created_unix_ms: manifest.created_unix_ms,
        updated_unix_ms: manifest.updated_unix_ms,
        default_standard: manifest.default_standard,
        default_background: manifest.default_background,
        pages,
        images,
        pdfs,
        next_stroke_id: manifest.next_stroke_id,
        next_page_id: manifest.next_page_id,
        next_image_id: manifest.next_image_id,
        page_gap_pt: manifest.page_gap_pt,
    })
}

fn read_entry_capped<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    total_read: &mut u64,
) -> Result<Vec<u8>, CoreError> {
    let entry = archive.by_name(name)?;
    let remaining = MAX_ARCHIVE_UNCOMPRESSED_BYTES.saturating_sub(*total_read);
    if entry.size() > remaining {
        return Err(CoreError::ArchiveTooLarge);
    }
    let cap_limit = remaining.saturating_add(1);
    let mut buf = Vec::with_capacity(entry.size().min(16 * 1024 * 1024) as usize);
    entry.take(cap_limit).read_to_end(&mut buf)?;
    let actual = buf.len() as u64;
    if actual > remaining {
        return Err(CoreError::ArchiveTooLarge);
    }
    *total_read = total_read.saturating_add(actual);
    Ok(buf)
}

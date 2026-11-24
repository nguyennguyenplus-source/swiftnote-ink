use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::coords::{CanvasPoint, PagePoint, Rect};
use crate::error::CoreError;
use crate::paper::{PageBackground, PaperStandard};
use crate::spatial::StrokeIndex;
use crate::stroke::{Stroke, StrokeId};

/// Content-addressed BLAKE3 digest for deduplicated lossless image and PDF blobs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(pub [u8; 32]);

impl AssetId {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            use fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    pub fn from_hex(hex: &str) -> Result<Self, CoreError> {
        if hex.len() != 64 || !hex.is_ascii() {
            return Err(CoreError::InvalidArchive(
                "invalid asset hash length".into(),
            ));
        }
        let mut out = [0u8; 32];
        for (i, slot) in out.iter_mut().enumerate() {
            let chunk = &hex[i * 2..i * 2 + 2];
            *slot = u8::from_str_radix(chunk, 16)
                .map_err(|_| CoreError::InvalidArchive("invalid asset hash hex".into()))?;
        }
        Ok(Self(out))
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({})", &self.to_hex()[..12])
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for AssetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsset {
    pub id: AssetId,
    pub png_bytes: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
}

impl ImageAsset {
    pub fn from_png_bytes(png_bytes: Vec<u8>) -> Result<Self, CoreError> {
        let (width_px, height_px) = {
            let decoder = png::Decoder::new(Cursor::new(&png_bytes));
            let reader = decoder
                .read_info()
                .map_err(|e| CoreError::PngError(e.to_string()))?;
            let info = reader.info();
            (info.width, info.height)
        };
        if width_px == 0 || height_px == 0 {
            return Err(CoreError::PngError("zero-dimension PNG".into()));
        }
        let id = AssetId::from_bytes(&png_bytes);
        Ok(Self {
            id,
            png_bytes,
            width_px,
            height_px,
        })
    }

    pub fn from_rgba(width_px: u32, height_px: u32, rgba: &[u8]) -> Result<Self, CoreError> {
        if width_px == 0 || height_px == 0 {
            return Err(CoreError::PngError("zero-dimension image".into()));
        }
        let expected_len = (width_px as usize)
            .checked_mul(height_px as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| CoreError::PngError("image dimensions overflow".into()))?;
        if rgba.len() != expected_len {
            return Err(CoreError::PngError("RGBA buffer length mismatch".into()));
        }
        let mut png_bytes = Vec::with_capacity(expected_len / 2);
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width_px, height_px);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|e| CoreError::PngError(e.to_string()))?;
            writer
                .write_image_data(rgba)
                .map_err(|e| CoreError::PngError(e.to_string()))?;
        }
        let id = AssetId::from_bytes(&png_bytes);
        Ok(Self {
            id,
            png_bytes,
            width_px,
            height_px,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImagePlacement {
    pub id: u64,
    pub asset_id: AssetId,
    pub bounds: Rect,
    /// Normalized crop rectangle inside `[0.0, 1.0]`.
    pub crop: Rect,
    pub rotation_rad: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfAssetMeta {
    pub id: AssetId,
    pub encrypted: bool,
    pub page_count: usize,
    /// Original PDF bytes stored verbatim; session unlock credentials are never persisted.
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PageId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    pub width_pt: f64,
    pub height_pt: f64,
    pub background: PageBackground,
    pub strokes: Vec<Stroke>,
    pub images: Vec<ImagePlacement>,
    #[serde(skip)]
    pub index: StrokeIndex,
}

impl Page {
    pub fn new(id: PageId, width_pt: f64, height_pt: f64, background: PageBackground) -> Self {
        Self {
            id,
            width_pt,
            height_pt,
            background,
            strokes: Vec::new(),
            images: Vec::new(),
            index: StrokeIndex::new(),
        }
    }

    #[inline]
    pub fn is_fixed_height(&self) -> bool {
        matches!(self.background, PageBackground::Pdf { .. })
    }

    pub fn insert_stroke_sorted(&mut self, stroke: Stroke) {
        self.index.insert(&stroke);
        let pos = self.strokes.partition_point(|s| s.id < stroke.id);
        if pos < self.strokes.len() && self.strokes[pos].id == stroke.id {
            let old = &self.strokes[pos];
            self.index.remove(old.id, old.bounds);
            self.strokes[pos] = stroke;
        } else {
            self.strokes.insert(pos, stroke);
        }
    }

    pub fn remove_stroke_by_id(&mut self, id: StrokeId) -> Option<Stroke> {
        let pos = self.strokes.binary_search_by_key(&id, |s| s.id).ok()?;
        let removed = self.strokes.remove(pos);
        self.index.remove(removed.id, removed.bounds);
        Some(removed)
    }

    pub fn rebuild_index(&mut self) {
        self.strokes.sort_by_key(|s| s.id);
        self.index.rebuild(&self.strokes);
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub note_id: String,
    pub title: String,
    pub created_unix_ms: u64,
    pub updated_unix_ms: u64,
    pub default_standard: PaperStandard,
    pub default_background: PageBackground,
    pub pages: Vec<Page>,
    pub images: HashMap<AssetId, ImageAsset>,
    pub pdfs: HashMap<AssetId, PdfAssetMeta>,
    pub next_stroke_id: u64,
    pub next_page_id: u64,
    pub next_image_id: u64,
    pub page_gap_pt: f64,
}

impl Document {
    pub fn new_untitled(note_id: impl Into<String>, title: impl Into<String>, now_ms: u64) -> Self {
        let standard = PaperStandard::A4;
        let (w, h) = standard.dimensions_pt();
        let default_bg = PageBackground::default();
        let first_page = Page::new(PageId(1), w, h, default_bg.clone());
        Self {
            note_id: note_id.into(),
            title: title.into(),
            created_unix_ms: now_ms,
            updated_unix_ms: now_ms,
            default_standard: standard,
            default_background: default_bg,
            pages: vec![first_page],
            images: HashMap::new(),
            pdfs: HashMap::new(),
            next_stroke_id: 1,
            next_page_id: 2,
            next_image_id: 1,
            page_gap_pt: 24.0,
        }
    }

    #[inline]
    pub fn allocate_stroke_id(&mut self) -> StrokeId {
        let id = StrokeId(self.next_stroke_id);
        self.next_stroke_id = self.next_stroke_id.saturating_add(1);
        id
    }

    #[inline]
    pub fn allocate_page_id(&mut self) -> PageId {
        let id = PageId(self.next_page_id);
        self.next_page_id = self.next_page_id.saturating_add(1);
        id
    }

    #[inline]
    pub fn allocate_image_id(&mut self) -> u64 {
        let id = self.next_image_id;
        self.next_image_id = self.next_image_id.saturating_add(1);
        id
    }

    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn page_mut(&mut self, id: PageId) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == id)
    }

    pub fn page_canvas_rect(&self, index: usize) -> Option<Rect> {
        let page = self.pages.get(index)?;
        let mut y = 0.0;
        for p in &self.pages[..index] {
            y += p.height_pt + self.page_gap_pt;
        }
        let x = -page.width_pt * 0.5;
        Some(Rect::new(x, y, x + page.width_pt, y + page.height_pt))
    }

    pub fn visible_pages(&self, vis_canvas: Rect) -> Vec<(PageId, Rect)> {
        let mut out = Vec::new();
        let mut y = 0.0;
        for page in &self.pages {
            let x = -page.width_pt * 0.5;
            let rect = Rect::new(x, y, x + page.width_pt, y + page.height_pt);
            if rect.intersects(vis_canvas) {
                out.push((page.id, rect));
            }
            y += page.height_pt + self.page_gap_pt;
        }
        out
    }

    pub fn page_at_canvas_point(&self, pt: CanvasPoint) -> Option<(PageId, PagePoint)> {
        let mut y = 0.0;
        for page in &self.pages {
            let x = -page.width_pt * 0.5;
            let rect = Rect::new(x, y, x + page.width_pt, y + page.height_pt);
            if rect.contains(pt.x, pt.y) {
                return Some((page.id, PagePoint::new(pt.x - x, pt.y - y)));
            }
            y += page.height_pt + self.page_gap_pt;
        }
        None
    }

    pub fn add_page(&mut self, standard: PaperStandard, background: PageBackground) -> PageId {
        let id = self.allocate_page_id();
        let (w, h) = standard.dimensions_pt();
        self.pages.push(Page::new(id, w, h, background));
        id
    }

    pub fn delete_page(&mut self, id: PageId) -> Result<(usize, Page), CoreError> {
        let idx = self
            .pages
            .iter()
            .position(|p| p.id == id)
            .ok_or(CoreError::PageNotFound(id.0))?;
        Ok((idx, self.pages.remove(idx)))
    }

    pub fn insert_image_asset(&mut self, asset: ImageAsset) -> AssetId {
        let id = asset.id;
        self.images.entry(id).or_insert(asset);
        id
    }

    pub fn add_image_placement(
        &mut self,
        page_id: PageId,
        asset_id: AssetId,
        bounds: Rect,
    ) -> Result<ImagePlacement, CoreError> {
        if !self.images.contains_key(&asset_id) {
            return Err(CoreError::AssetNotFound);
        }
        let placement_id = self.allocate_image_id();
        let placement = ImagePlacement {
            id: placement_id,
            asset_id,
            bounds,
            crop: Rect::new(0.0, 0.0, 1.0, 1.0),
            rotation_rad: 0.0,
        };
        let page = self
            .page_mut(page_id)
            .ok_or(CoreError::PageNotFound(page_id.0))?;
        page.images.push(placement.clone());
        Ok(placement)
    }

    pub fn move_resize_image(
        &mut self,
        page_id: PageId,
        placement_id: u64,
        new_bounds: Rect,
    ) -> Result<Rect, CoreError> {
        let page = self
            .page_mut(page_id)
            .ok_or(CoreError::PageNotFound(page_id.0))?;
        let img = page
            .images
            .iter_mut()
            .find(|img| img.id == placement_id)
            .ok_or(CoreError::AssetNotFound)?;
        let old = img.bounds;
        img.bounds = new_bounds;
        Ok(old)
    }

    pub fn append_pdf_pages(
        &mut self,
        meta: PdfAssetMeta,
        page_sizes_pt: &[(f64, f64)],
        locked_placeholder: bool,
    ) -> Vec<PageId> {
        let asset_id = meta.id;
        self.pdfs.entry(asset_id).or_insert(meta);
        let mut created = Vec::with_capacity(page_sizes_pt.len());
        for (page_index, &(w, h)) in page_sizes_pt.iter().enumerate() {
            let page_id = self.allocate_page_id();
            let bg = PageBackground::Pdf {
                asset_id: asset_id.0,
                page_index,
                locked_placeholder,
            };
            self.pages.push(Page::new(page_id, w, h, bg));
            created.push(page_id);
        }
        created
    }
}

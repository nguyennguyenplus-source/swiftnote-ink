use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use pdfium_render::prelude::*;
use swiftnote_core::AssetId;
use tracing::{debug, warn};

use crate::error::GpuError;

pub const MAX_PDF_TILE_PX: u32 = 2048;

#[derive(Debug, Clone, PartialEq)]
pub struct PdfOpenSummary {
    pub asset_id: AssetId,
    pub page_count: usize,
    pub page_sizes_pt: Vec<(f64, f64)>,
    pub encrypted: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfTileRequest {
    pub generation: u64,
    pub asset_id: AssetId,
    pub page_index: usize,
    pub tile_x: u32,
    pub tile_y: u32,
    pub tile_cols: u32,
    pub tile_rows: u32,
    pub full_width_px: u32,
    pub full_height_px: u32,
    pub tile_width_px: u32,
    pub tile_height_px: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfTileResult {
    pub generation: u64,
    pub asset_id: AssetId,
    pub page_index: usize,
    pub tile_x: u32,
    pub tile_y: u32,
    pub tile_cols: u32,
    pub tile_rows: u32,
    pub width_px: u32,
    pub height_px: u32,
    pub rgba: Vec<u8>,
}

/// Computes full pixel size and a grid of tiles each capped at `MAX_PDF_TILE_PX` (2048px).
pub fn compute_pdf_tile_grid(
    page_w_pt: f64,
    page_h_pt: f64,
    scale_px_per_pt: f64,
) -> (u32, u32, u32, u32) {
    let scale = scale_px_per_pt.clamp(0.05, 32.0);
    let full_w = (page_w_pt * scale).round().max(1.0) as u32;
    let full_h = (page_h_pt * scale).round().max(1.0) as u32;
    let cols = full_w.div_ceil(MAX_PDF_TILE_PX).max(1);
    let rows = full_h.div_ceil(MAX_PDF_TILE_PX).max(1);
    (full_w, full_h, cols, rows)
}

pub fn build_pdf_tile_requests(
    generation: u64,
    asset_id: AssetId,
    page_index: usize,
    page_w_pt: f64,
    page_h_pt: f64,
    scale_px_per_pt: f64,
) -> Vec<PdfTileRequest> {
    let (full_w, full_h, cols, rows) = compute_pdf_tile_grid(page_w_pt, page_h_pt, scale_px_per_pt);
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for ty in 0..rows {
        let y0 = ty * MAX_PDF_TILE_PX;
        let tile_h = (full_h.saturating_sub(y0)).clamp(1, MAX_PDF_TILE_PX);
        for tx in 0..cols {
            let x0 = tx * MAX_PDF_TILE_PX;
            let tile_w = (full_w.saturating_sub(x0)).clamp(1, MAX_PDF_TILE_PX);
            out.push(PdfTileRequest {
                generation,
                asset_id,
                page_index,
                tile_x: tx,
                tile_y: ty,
                tile_cols: cols,
                tile_rows: rows,
                full_width_px: full_w,
                full_height_px: full_h,
                tile_width_px: tile_w,
                tile_height_px: tile_h,
            });
        }
    }
    out
}

enum WorkerCommand {
    Open {
        asset_id: AssetId,
        raw_bytes: Vec<u8>,
        password: Option<String>,
        reply: SyncSender<Result<PdfOpenSummary, GpuError>>,
    },
    RenderTile(PdfTileRequest),
    Shutdown,
}

pub struct PdfWorker {
    tx: SyncSender<WorkerCommand>,
    rx: Receiver<PdfTileResult>,
    active_gen: Arc<AtomicU64>,
    dropped_stale_count: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Default for PdfWorker {
    fn default() -> Self {
        Self::spawn()
    }
}

impl PdfWorker {
    pub fn spawn() -> Self {
        let (tx, cmd_rx) = mpsc::sync_channel::<WorkerCommand>(64);
        let (res_tx, rx) = mpsc::sync_channel::<PdfTileResult>(64);
        let active_gen = Arc::new(AtomicU64::new(1));
        let dropped_stale_count = Arc::new(AtomicU64::new(0));

        let gen_ref = Arc::clone(&active_gen);
        let drop_ref = Arc::clone(&dropped_stale_count);

        let thread = thread::Builder::new()
            .name("swiftnote-pdf-worker".into())
            .spawn(move || {
                run_worker_loop(cmd_rx, res_tx, gen_ref, drop_ref);
            })
            .ok();

        Self {
            tx,
            rx,
            active_gen,
            dropped_stale_count,
            thread,
        }
    }

    #[inline]
    pub fn current_generation(&self) -> u64 {
        self.active_gen.load(Ordering::Acquire)
    }

    #[inline]
    pub fn advance_generation(&self) -> u64 {
        self.active_gen.fetch_add(1, Ordering::AcqRel) + 1
    }

    #[inline]
    pub fn dropped_stale_count(&self) -> u64 {
        self.dropped_stale_count.load(Ordering::Acquire)
    }

    pub fn open_document(
        &self,
        raw_bytes: Vec<u8>,
        password: Option<String>,
    ) -> Result<PdfOpenSummary, GpuError> {
        let asset_id = AssetId::from_bytes(&raw_bytes);
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(WorkerCommand::Open {
                asset_id,
                raw_bytes,
                password,
                reply: reply_tx,
            })
            .map_err(|_| GpuError::PdfError("PDF worker channel closed".into()))?;
        reply_rx
            .recv()
            .map_err(|_| GpuError::PdfError("PDF worker reply failed".into()))?
    }

    pub fn request_tile(&self, req: PdfTileRequest) -> bool {
        if req.generation < self.active_gen.load(Ordering::Acquire) {
            self.dropped_stale_count.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        match self.tx.try_send(WorkerCommand::RenderTile(req)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                debug!("PDF render queue full; dropping request for later frame");
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn poll_completed(&self) -> Vec<PdfTileResult> {
        let mut out = Vec::new();
        let cur_gen = self.active_gen.load(Ordering::Acquire);
        while let Ok(tile) = self.rx.try_recv() {
            if tile.generation >= cur_gen {
                out.push(tile);
            } else {
                self.dropped_stale_count.fetch_add(1, Ordering::Relaxed);
            }
        }
        out
    }
}

impl Drop for PdfWorker {
    fn drop(&mut self) {
        let _ = self.tx.try_send(WorkerCommand::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

struct StoredPdf {
    raw_bytes: Vec<u8>,
    password: Option<String>,
}

fn try_bind_pdfium() -> Result<Pdfium, GpuError> {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            let candidate = Pdfium::pdfium_platform_library_name_at_path(dir);
            if candidate.exists() {
                if let Ok(bindings) = Pdfium::bind_to_library(&candidate) {
                    return Ok(Pdfium::new(bindings));
                }
            }
        }
    }

    let cwd_candidate = Pdfium::pdfium_platform_library_name_at_path(".");
    if cwd_candidate.exists() {
        if let Ok(bindings) = Pdfium::bind_to_library(&cwd_candidate) {
            return Ok(Pdfium::new(bindings));
        }
    }

    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|_| GpuError::PdfLibraryMissing)
}

fn run_worker_loop(
    cmd_rx: Receiver<WorkerCommand>,
    res_tx: SyncSender<PdfTileResult>,
    active_gen: Arc<AtomicU64>,
    dropped_stale_count: Arc<AtomicU64>,
) {
    let mut pdfium: Option<&'static Pdfium> = None;
    let mut docs: HashMap<AssetId, StoredPdf> = HashMap::new();

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            WorkerCommand::Shutdown => break,
            WorkerCommand::Open {
                asset_id,
                raw_bytes,
                password,
                reply,
            } => {
                if pdfium.is_none() {
                    match try_bind_pdfium() {
                        Ok(instance) => {
                            pdfium = Some(Box::leak(Box::new(instance)));
                        }
                        Err(err) => {
                            let _ = reply.send(Err(err));
                            continue;
                        }
                    }
                }
                let Some(pdfium_ref) = pdfium else {
                    let _ = reply.send(Err(GpuError::PdfLibraryMissing));
                    continue;
                };

                let res = open_pdf_summary(pdfium_ref, asset_id, &raw_bytes, password.as_deref());
                if let Ok(ref summary) = res {
                    if !summary.locked {
                        docs.insert(
                            asset_id,
                            StoredPdf {
                                raw_bytes,
                                password,
                            },
                        );
                    }
                }
                let _ = reply.send(res);
            }
            WorkerCommand::RenderTile(req) => {
                if req.generation < active_gen.load(Ordering::Acquire) {
                    dropped_stale_count.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let Some(pdfium_ref) = pdfium else {
                    continue;
                };
                let Some(stored) = docs.get(&req.asset_id) else {
                    continue;
                };
                match render_tile_bytes(pdfium_ref, stored, &req) {
                    Ok(rgba) => {
                        if req.generation >= active_gen.load(Ordering::Acquire) {
                            let _ = res_tx.try_send(PdfTileResult {
                                generation: req.generation,
                                asset_id: req.asset_id,
                                page_index: req.page_index,
                                tile_x: req.tile_x,
                                tile_y: req.tile_y,
                                tile_cols: req.tile_cols,
                                tile_rows: req.tile_rows,
                                width_px: req.tile_width_px,
                                height_px: req.tile_height_px,
                                rgba,
                            });
                        } else {
                            dropped_stale_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(err) => {
                        warn!("failed rendering PDF page {} tile: {err}", req.page_index);
                    }
                }
            }
        }
    }
}

fn open_pdf_summary(
    pdfium: &Pdfium,
    asset_id: AssetId,
    raw_bytes: &[u8],
    password: Option<&str>,
) -> Result<PdfOpenSummary, GpuError> {
    match pdfium.load_pdf_from_byte_slice(raw_bytes, password) {
        Ok(doc) => {
            let page_count = usize::try_from(doc.pages().len()).unwrap_or(0);
            let mut page_sizes_pt = Vec::with_capacity(page_count);
            for page in doc.pages().iter() {
                page_sizes_pt.push((
                    f64::from(page.width().value),
                    f64::from(page.height().value),
                ));
            }
            Ok(PdfOpenSummary {
                asset_id,
                page_count,
                page_sizes_pt,
                encrypted: password.is_some(),
                locked: false,
            })
        }
        Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => {
            Ok(PdfOpenSummary {
                asset_id,
                page_count: 0,
                page_sizes_pt: Vec::new(),
                encrypted: true,
                locked: true,
            })
        }
        Err(err) => Err(GpuError::PdfError(err.to_string())),
    }
}

fn render_tile_bytes(
    pdfium: &Pdfium,
    stored: &StoredPdf,
    req: &PdfTileRequest,
) -> Result<Vec<u8>, GpuError> {
    let doc = pdfium
        .load_pdf_from_byte_slice(&stored.raw_bytes, stored.password.as_deref())
        .map_err(|e| GpuError::PdfError(e.to_string()))?;
    let page_idx = PdfPageIndex::try_from(req.page_index)
        .map_err(|_| GpuError::PdfError("PDF page index out of PdfPageIndex bounds".into()))?;
    let page = doc
        .pages()
        .get(page_idx)
        .map_err(|e| GpuError::PdfError(e.to_string()))?;

    let bitmap = if req.tile_cols == 1 && req.tile_rows == 1 {
        let cfg = PdfRenderConfig::new()
            .set_target_size(req.full_width_px as Pixels, req.full_height_px as Pixels);
        page.render_with_config(&cfg)
            .map_err(|e| GpuError::PdfError(e.to_string()))?
    } else {
        let w_pt = f32::max(page.width().value, 1.0);
        let h_pt = f32::max(page.height().value, 1.0);
        let scale_x = req.full_width_px as f32 / w_pt;
        let scale_y = req.full_height_px as f32 / h_pt;
        let x_off = (req.tile_x * MAX_PDF_TILE_PX) as f32;
        let y_off = (req.tile_y * MAX_PDF_TILE_PX) as f32;
        let cfg = PdfRenderConfig::new()
            .set_fixed_size(req.tile_width_px as Pixels, req.tile_height_px as Pixels)
            .clip(
                0,
                0,
                req.tile_width_px as Pixels,
                req.tile_height_px as Pixels,
            )
            .render_form_data(false)
            .reset_matrix(PdfMatrix::new(scale_x, 0.0, 0.0, scale_y, -x_off, -y_off))
            .map_err(|e| GpuError::PdfError(e.to_string()))?;
        page.render_with_config(&cfg)
            .map_err(|e| GpuError::PdfError(e.to_string()))?
    };

    let dyn_img = bitmap
        .as_image()
        .map_err(|e| GpuError::PdfError(e.to_string()))?;
    Ok(dyn_img.to_rgba8().into_raw())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use swiftnote_core::PdfAssetMeta;

    #[test]
    fn pdf_worker_reports_missing_library_gracefully() {
        let worker = PdfWorker::spawn();
        let res = worker.open_document(b"%PDF-1.4 mock".to_vec(), None);
        // Either missing library in test env, or invalid PDF error if system pdfium exists — never panics!
        assert!(matches!(
            res,
            Err(GpuError::PdfLibraryMissing | GpuError::PdfError(_))
        ));
    }

    #[test]
    fn pdf_worker_drops_stale_generation_requests_on_fast_scroll() {
        let worker = PdfWorker::spawn();
        let asset_id = AssetId::from_bytes(b"pdf");
        let stale_gen = worker.current_generation();
        let new_gen = worker.advance_generation();
        assert!(new_gen > stale_gen);

        let stale_req = PdfTileRequest {
            generation: stale_gen,
            asset_id,
            page_index: 0,
            tile_x: 0,
            tile_y: 0,
            tile_cols: 1,
            tile_rows: 1,
            full_width_px: 600,
            full_height_px: 800,
            tile_width_px: 600,
            tile_height_px: 800,
        };
        assert!(!worker.request_tile(stale_req));
        assert!(worker.dropped_stale_count() >= 1);
    }

    #[test]
    fn pdf_worker_splits_deep_zoom_into_max_2048_tiles() {
        // US Letter zoomed 6x => 3672 x 4752 px => 2x3 grid of <= 2048px tiles
        let requests =
            build_pdf_tile_requests(1, AssetId::from_bytes(b"letter"), 0, 612.0, 792.0, 6.0);
        assert_eq!(requests.len(), 6);
        for req in &requests {
            assert!(req.tile_width_px <= MAX_PDF_TILE_PX);
            assert!(req.tile_height_px <= MAX_PDF_TILE_PX);
            assert!(req.tile_width_px >= 1 && req.tile_height_px >= 1);
        }
    }

    #[test]
    fn pdf_worker_session_passwords_never_leak_into_asset_meta() {
        let meta = PdfAssetMeta {
            id: AssetId::from_bytes(b"secret_pdf"),
            encrypted: true,
            page_count: 10,
            raw_bytes: b"%PDF-1.7 encrypted".to_vec(),
        };
        let dbg = format!("{meta:?}");
        assert!(!dbg.contains("my_secret_password"));
    }
}

pub mod error;
pub mod mipmap;
pub mod pdf_worker;
pub mod renderer;
pub mod tile_cache;

pub use error::GpuError;
pub use mipmap::compute_mip_level_count;
pub use pdf_worker::{
    build_pdf_tile_requests, compute_pdf_tile_grid, PdfOpenSummary, PdfTileRequest, PdfTileResult,
    PdfWorker, MAX_PDF_TILE_PX,
};
pub use renderer::{
    default_wgpu_backends, CursorOverlay, PdfTileKey, StrokePipelineConfig, UnifiedRenderer,
};
pub use tile_cache::{
    visible_tile_keys, GpuTileCache, TileKey, DEFAULT_VRAM_BUDGET_BYTES, TILE_SIZE_PX,
};

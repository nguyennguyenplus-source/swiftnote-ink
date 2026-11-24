#![forbid(unsafe_code)]

pub mod brush;
pub mod contour;
pub mod coords;
pub mod document;
pub mod eraser;
pub mod error;
pub mod history;
pub mod library;
pub mod paper;
pub mod settings;
pub mod spatial;
pub mod stabilizer;
pub mod storage;
pub mod stroke;
pub mod tool;

pub use brush::{default_pen_slots, BrushKind, BrushStyle, Color, DEFAULT_PALETTE};
pub use contour::{ContourMesh, ContourVertex, StrokeTessellator};
pub use coords::{CanvasPoint, PagePoint, PhysicalPoint, Rect, ScreenPoint, ViewportTransform};
pub use document::{AssetId, Document, ImageAsset, ImagePlacement, Page, PageId, PdfAssetMeta};
pub use eraser::{erase_at, EraseResult, EraserMode, EraserSize};
pub use error::{CoreError, MAX_ARCHIVE_UNCOMPRESSED_BYTES};
pub use history::{DirtyRegion, UndoCommand, UndoStack};
pub use library::{now_unix_ms, LibraryEntry, LibraryIndex, NoteLibrary};
pub use paper::{PageBackground, PaperPolarity, PaperStandard, PaperTemplate};
pub use settings::{AppSettings, InputMode, StabilizerPreset, UiTheme};
pub use spatial::StrokeIndex;
pub use stabilizer::{
    decompose_pressure_step, InputGuard, RawInputSample, StrokeStabilizer, MAX_PROXIMITY_JUMP_PX,
    MAX_WIDTH_VARIATION, PIXEL_MOTION_THRESHOLD,
};
pub use storage::{load_swnote, save_swnote_atomic};
pub use stroke::{Stroke, StrokeId, StrokePoint};
pub use tool::{ActiveTool, ToolState};

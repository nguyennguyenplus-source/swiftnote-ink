#![forbid(unsafe_code)]

pub mod brush;
pub mod contour;
pub mod coords;
pub mod error;
pub mod paper;
pub mod settings;
pub mod stabilizer;
pub mod stroke;

pub use brush::{default_pen_slots, BrushKind, BrushStyle, Color, DEFAULT_PALETTE};
pub use contour::{ContourMesh, ContourVertex, StrokeTessellator};
pub use coords::{CanvasPoint, PagePoint, PhysicalPoint, Rect, ScreenPoint, ViewportTransform};
pub use error::{CoreError, MAX_ARCHIVE_UNCOMPRESSED_BYTES};
pub use paper::{PageBackground, PaperPolarity, PaperStandard, PaperTemplate};
pub use settings::{AppSettings, InputMode, StabilizerPreset, UiTheme};
pub use stabilizer::{
    decompose_pressure_step, InputGuard, RawInputSample, StrokeStabilizer, MAX_PROXIMITY_JUMP_PX,
    MAX_WIDTH_VARIATION, PIXEL_MOTION_THRESHOLD,
};
pub use stroke::{Stroke, StrokeId, StrokePoint};

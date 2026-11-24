#![forbid(unsafe_code)]

pub mod brush;
pub mod coords;
pub mod error;
pub mod paper;
pub mod settings;

pub use brush::{default_pen_slots, BrushKind, BrushStyle, Color, DEFAULT_PALETTE};
pub use coords::{CanvasPoint, PagePoint, PhysicalPoint, Rect, ScreenPoint, ViewportTransform};
pub use error::{CoreError, MAX_ARCHIVE_UNCOMPRESSED_BYTES};
pub use paper::{PageBackground, PaperPolarity, PaperStandard, PaperTemplate};
pub use settings::{AppSettings, InputMode, StabilizerPreset, UiTheme};

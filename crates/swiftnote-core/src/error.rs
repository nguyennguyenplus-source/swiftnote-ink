use thiserror::Error;

/// Hard decompression cap for `.swnote` ZIP archives (256 MiB) to guard against zip bombs.
pub const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("non-finite or invalid coordinate values")]
    InvalidCoordinate,

    #[error("page {0} not found")]
    PageNotFound(u64),

    #[error("asset not found in document store")]
    AssetNotFound,

    #[error("note {0} not found in library")]
    NoteNotFound(String),

    #[error("corrupt or unsupported .swnote archive: {0}")]
    InvalidArchive(String),

    #[error("archive exceeded uncompressed safety cap of {MAX_ARCHIVE_UNCOMPRESSED_BYTES} bytes")]
    ArchiveTooLarge,

    #[error("PNG image encoding/decoding failed: {0}")]
    PngError(String),

    #[error("serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("archive I/O failed: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("filesystem I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

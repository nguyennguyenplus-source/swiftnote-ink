use thiserror::Error;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("no suitable GPU adapter found")]
    NoAdapter,

    #[error("failed to create GPU device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),

    #[error("failed to create GPU surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),

    #[error("image decoding failed: {0}")]
    ImageDecode(String),

    #[error("PDFium runtime library not found alongside executable or on system path")]
    PdfLibraryMissing,

    #[error("PDF password required to unlock encrypted document")]
    PdfPasswordRequired,

    #[error("failed to open or render PDF document: {0}")]
    PdfError(String),
}

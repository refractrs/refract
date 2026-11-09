//! Errors at the native host boundary.

/// Errors are returned to the launcher rather than leaving a partly updated host alive.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("{0}")]
    Core(#[from] refract_core::ViewError),
    #[error("native layout: {0}")]
    Layout(#[from] taffy::TaffyError),
    #[error("native host: {0}")]
    Host(String),
    #[error("native renderer: {0}")]
    Render(String),
    #[error("native does not yet support {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, NativeError>;

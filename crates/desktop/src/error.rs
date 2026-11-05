//! Errors at the native host boundary.

/// Errors are returned to the launcher rather than leaving a partly updated host alive.
#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("{0}")]
    Core(#[from] refract_core::ViewError),
    #[error("desktop layout: {0}")]
    Layout(#[from] taffy::TaffyError),
    #[error("desktop host: {0}")]
    Host(String),
    #[error("desktop renderer: {0}")]
    Render(String),
    #[error("desktop does not yet support {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, DesktopError>;

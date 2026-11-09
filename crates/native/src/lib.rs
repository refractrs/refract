//! Native Refract hosting: retained patches, Taffy layout, Parley text, and replaceable GPU rendering.
#![cfg(not(target_arch = "wasm32"))]

mod app;
mod editor;
mod error;
mod host;
mod image;
mod layout;
mod platform;
pub mod renderer;
mod scheduler;
mod surface;
mod text;

pub use error::{NativeError, Result};

#[cfg(feature = "vello")]
pub use app::run;
pub use app::{run_with_renderer, NativeOptions};
pub use host::HostStats;
pub use scheduler::{NativeHandle, ScheduleError, TimerGuard};
/// The window type passed to custom renderer factories.
pub use winit::window::Window;

#[cfg(test)]
mod tests;

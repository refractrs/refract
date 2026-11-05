//! DOM adapter for Refract. Browser objects never enter the portable core.
#[cfg(target_arch = "wasm32")]
mod dom;
#[cfg(target_arch = "wasm32")]
mod image;
#[cfg(any(test, target_arch = "wasm32"))]
mod style;
#[cfg(target_arch = "wasm32")]
pub use dom::*;

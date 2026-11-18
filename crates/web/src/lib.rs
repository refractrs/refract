//! DOM adapter for Refract. Browser objects never enter the portable core.
#[cfg(target_arch = "wasm32")]
mod dom;
#[cfg(target_arch = "wasm32")]
mod rich_text;
mod html;
#[cfg(target_arch = "wasm32")]
mod image;
mod prerender;
#[cfg(target_arch = "wasm32")]
mod shader;
mod style;
mod svg;
#[cfg(target_arch = "wasm32")]
pub use dom::*;
#[doc(hidden)]
pub use prerender::encode_export;
pub use prerender::{prerender, prerender_with_inputs, PrerenderOptions};

//! Refract: composable Rust UI with a portable reactive core.
pub use refract_core::*;
#[cfg(feature = "macros")]
pub use refract_macros::view;
/// Common types, constructors, and optional macros for writing Refract views.
pub mod prelude {
    pub use refract_core::prelude::*;
    #[cfg(feature = "macros")]
    pub use refract_macros::view;
}

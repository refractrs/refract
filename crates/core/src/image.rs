//! Portable image descriptions. Encoded bytes belong to application assets;
//! decoding, I/O, and GPU resource ownership belong to adapters.

use std::sync::Arc;

/// Encodings supported by embedded images on both runtimes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
        }
    }
}

/// An image location or immutable encoded asset. Clones share the asset bytes.
/// Keep embedded sources in a model or static LazyLock to avoid reallocating them
/// during view evaluation. URL strings remain accepted by the `image` builder.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ImageSource {
    /// Browser URL; on desktop, HTTP(S) URL or path relative to the working directory.
    Url(Arc<str>),
    Embedded {
        format: ImageFormat,
        bytes: Arc<[u8]>,
    },
}

impl ImageSource {
    pub fn embedded(format: ImageFormat, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self::Embedded {
            format,
            bytes: bytes.into(),
        }
    }

    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Url(url) => Some(url),
            Self::Embedded { .. } => None,
        }
    }
}

// Logging an element should never dump an entire encoded photograph.
impl std::fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Url(url) => f.debug_tuple("Url").field(url).finish(),
            Self::Embedded { format, bytes } => f
                .debug_struct("Embedded")
                .field("format", format)
                .field("bytes", &bytes.len())
                .finish(),
        }
    }
}

impl From<String> for ImageSource {
    fn from(value: String) -> Self {
        Self::Url(value.into())
    }
}
impl From<&str> for ImageSource {
    fn from(value: &str) -> Self {
        Self::Url(value.into())
    }
}
impl From<&String> for ImageSource {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

/// How pixels fit an image's content box. Images are centered in both axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFit {
    /// Preserve aspect ratio and show the whole image, leaving empty space if necessary.
    #[default]
    Contain,
    /// Preserve aspect ratio and crop to fill the content box.
    Cover,
    /// Fill the content box without preserving aspect ratio.
    Stretch,
}

//! Browser-owned decoding. Embedded assets use object URLs whose lifetime follows
//! their retained element; replacing/removing an image revokes the previous URL.

use refract_core::ImageSource;
use wasm_bindgen::JsValue;
use web_sys::{Blob, BlobPropertyBag, Url};

pub(crate) struct ImageUrl {
    url: String,
    // Only URLs created from embedded bytes belong to this adapter.
    owned: bool,
}

impl ImageUrl {
    pub fn new(source: &ImageSource) -> Result<Self, JsValue> {
        match source {
            ImageSource::Url(url) => Ok(Self {
                url: url.to_string(),
                owned: false,
            }),
            ImageSource::Embedded { format, bytes } => {
                let parts = js_sys::Array::new();
                parts.push(&js_sys::Uint8Array::from(bytes.as_ref()));
                let options = BlobPropertyBag::new();
                options.set_type(format.mime_type());
                let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
                Ok(Self {
                    url: Url::create_object_url_with_blob(&blob)?,
                    owned: true,
                })
            }
        }
    }

    pub fn as_str(&self) -> &str {
        &self.url
    }
}

impl Drop for ImageUrl {
    fn drop(&mut self) {
        if self.owned {
            let _ = Url::revoke_object_url(&self.url);
        }
    }
}

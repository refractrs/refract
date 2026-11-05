//! Optional browser conformance fixture, excluded from normal gallery builds.
use super::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct ImageFixture {
    mount: refract_web::Mount,
    source: Binding<ImageSource>,
    alt: Binding<Option<String>>,
}

#[wasm_bindgen]
impl ImageFixture {
    pub fn set_alt(&self, value: Option<String>) -> Result<(), JsValue> {
        let binding = self.alt.clone();
        self.mount
            .handle()
            .update(move |rt| binding.set_if_changed(rt, value))
    }

    pub fn select(&self, index: usize) -> Result<(), JsValue> {
        let source = if index < PHOTOS.len() {
            PHOTOS[index].source.clone()
        } else {
            ImageSource::embedded(ImageFormat::Png, b"invalid image".as_slice())
        };
        let binding = self.source.clone();
        self.mount
            .handle()
            .update(move |rt| binding.set_if_changed(rt, source))
    }
    pub fn use_url(&self, url: String) -> Result<(), JsValue> {
        let binding = self.source.clone();
        self.mount
            .handle()
            .update(move |rt| binding.set_if_changed(rt, url.into()))
    }
    pub fn dispose(&self) {
        self.mount.dispose();
    }
    pub fn last_error(&self) -> Option<String> {
        self.mount.last_error()
    }
}

#[wasm_bindgen]
pub fn start_image_fixture(host: web_sys::Element) -> Result<ImageFixture, JsValue> {
    let captured = Rc::new(RefCell::new(None));
    let view = view_fn({
        let captured = captured.clone();
        move |cx| {
            let source = cx.state("image-source", || PHOTOS[0].source.clone());
            let alt = cx.state("image-alt", || None::<String>);
            *captured.borrow_mut() = Some((source.clone(), alt.clone()));
            let image = image(cx.read(&source)).width(120.0).height(80.0);
            Ok(match cx.read(&alt) {
                Some(value) => image.label(value),
                None => image,
            })
        }
    });
    let mount = refract_web::mount(host, view, refract_web::MountOptions::default())?;
    let (source, alt) = captured.borrow().clone().expect("initial view was mounted");
    Ok(ImageFixture { mount, source, alt })
}

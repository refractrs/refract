//! One gallery view for the browser and desktop. The assets are compiled into the
//! application: no working-directory assumptions, remote requests, or network setup.

use refract::prelude::*;
use std::sync::LazyLock;

pub const BACKGROUND: Color = Color::rgb(17, 19, 21);
const SURFACE: Color = Color::rgb(25, 28, 30);
const INK: Color = Color::rgb(237, 237, 228);
const MUTED: Color = Color::rgb(147, 155, 151);
const LINE: Color = Color::rgb(48, 54, 51);
const ACCENT: Color = Color::rgb(190, 210, 169);
const SERIF: &str = "Georgia, serif";
const MONO: &str = "'SFMono-Regular', Consolas, monospace";

struct Photo {
    source: ImageSource,
    number: &'static str,
    title: &'static str,
    caption: &'static str,
    credit: &'static str,
    alt: &'static str,
}

static PHOTOS: LazyLock<[Photo; 3]> = LazyLock::new(|| {
    [
        Photo {
            source: ImageSource::embedded(
                ImageFormat::Jpeg,
                include_bytes!("../assets/alpine.jpg").as_slice(),
            ),
            number: "01",
            title: "Alpine silence",
            caption: "Still water. Endless sky.",
            credit: "Brittany Martin",
            alt: "A turquoise alpine lake reflecting mountains and evergreen trees",
        },
        Photo {
            source: ImageSource::embedded(
                ImageFormat::Jpeg,
                include_bytes!("../assets/dunes.jpg").as_slice(),
            ),
            number: "02",
            title: "Shaped by the wind",
            caption: "The desert, in slow motion.",
            credit: "Martin Sanchez",
            alt: "Warm orange desert dunes with sweeping curves and deep shadows",
        },
        Photo {
            source: ImageSource::embedded(
                ImageFormat::Jpeg,
                include_bytes!("../assets/forest.jpg").as_slice(),
            ),
            number: "03",
            title: "Into the quiet",
            caption: "Where the forest meets the fog.",
            credit: "Sabin Popa",
            alt: "Layers of dark green forest disappearing into pale morning mist",
        },
    ]
});

#[view]
fn photograph(cx: &mut ViewContext<'_>, index: usize, fit: Binding<ImageFit>) -> impl View {
    let photo = &PHOTOS[index];
    image(photo.source.clone())
        .label(photo.alt)
        .fit(cx.read(&fit))
        .fill_width()
        .height(330.0)
        .background(SURFACE)
        .radius(8.0)
}

fn card(index: usize, fit: Binding<ImageFit>) -> impl View {
    let photo = &PHOTOS[index];
    v_stack()
        .gap(20.0)
        .child(photograph(index, fit))
        .child(
            v_stack()
                .gap(8.0)
                .child(
                    h_stack()
                        .justify(Alignment::SpaceBetween)
                        .alignment(Alignment::Center)
                        .child(
                            text(photo.title)
                                .font_family(SERIF)
                                .font_size(24.0)
                                .foreground(INK),
                        )
                        .child(
                            text(photo.number)
                                .font_family(MONO)
                                .font_size(11.0)
                                .foreground(MUTED),
                        ),
                )
                .child(text(photo.caption).font_size(13.0).foreground(MUTED))
                .child(
                    text(format!("Photograph by {}", photo.credit))
                        .font_size(11.0)
                        .foreground(MUTED),
                ),
        )
        .grow(1.0)
        .min_width(0.0)
}

#[view]
fn fit_control(cx: &mut ViewContext<'_>, fit: Binding<ImageFit>) -> impl View {
    let covering = cx.read(&fit) == ImageFit::Cover;
    #[action(fit)]
    let toggle_fit = |cx| {
        fit.update(cx.runtime(), |value| {
            *value = if *value == ImageFit::Cover {
                ImageFit::Contain
            } else {
                ImageFit::Cover
            };
        })
    };
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::Center)
        .child(
            text(if covering {
                "Show full images"
            } else {
                "Fill image frames"
            })
            .font_size(12.0),
        )
        .on_press(toggle_fit)
        .label(if covering {
            "Show full images"
        } else {
            "Fill image frames"
        })
        .width(156.0)
        .height(34.0)
        .foreground(INK)
        .background(SURFACE)
        .border(Border::all(1.0, LINE))
        .radius(17.0)
}

#[view]
fn gallery(cx: &mut ViewContext<'_>) -> impl View {
    #[state]
    let fit: ImageFit = ImageFit::Cover;

    v_stack()
        .gap(36.0)
        .child(
            h_stack()
                .justify(Alignment::SpaceBetween)
                .alignment(Alignment::Center)
                .child(
                    h_stack()
                        .gap(12.0)
                        .alignment(Alignment::Center)
                        .child(
                            text("r")
                                .font_family(SERIF)
                                .font_size(32.0)
                                .foreground(ACCENT),
                        )
                        .child(
                            text("refract / gallery")
                                .font_size(14.0)
                                .font_weight(600)
                                .foreground(INK),
                        ),
                )
                .child(
                    text("COLLECTION  /  003")
                        .font_family(MONO)
                        .font_size(10.0)
                        .foreground(MUTED),
                ),
        )
        .child(
            v_stack()
                .gap(14.0)
                .child(
                    text("Somewhere quieter.")
                        .heading(1)
                        .font_family(SERIF)
                        .font_size(54.0)
                        .line_height(64.0)
                        .foreground(INK),
                )
                .child(
                    text("Three views worth getting lost in.")
                        .font_size(16.0)
                        .foreground(MUTED),
                ),
        )
        .child(
            v_stack()
                .gap(22.0)
                .child(
                    h_stack()
                        .justify(Alignment::SpaceBetween)
                        .alignment(Alignment::Center)
                        .child(
                            text("COLLECTED LANDSCAPES")
                                .font_family(MONO)
                                .font_size(10.0)
                                .foreground(ACCENT),
                        )
                        .child(fit_control(fit.clone())),
                )
                .child(
                    h_stack()
                        .gap(22.0)
                        .children((0..3).map(|index| card(index, fit.clone()).key(index)))
                        .min_width(720.0)
                        .scrollable().axis(Axis::Horizontal)
                        .fill_width(),
                ),
        )
        .child(
            v_stack()
                .gap(18.0)
                .child(view().height(1.0).background(LINE))
                .child(
                    h_stack()
                        .justify(Alignment::SpaceBetween)
                        .child(
                            text("A little space to look around.")
                                .font_size(12.0)
                                .foreground(MUTED),
                        )
                        .child(
                            text("PHOTOS / UNSPLASH")
                                .font_family(MONO)
                                .font_size(9.0)
                                .foreground(MUTED),
                        ),
                ),
        )
        .padding(36.0)
        .background(BACKGROUND)
        .fill_width()
        .scrollable()
        .fill_width()
        .fill_height()
}

pub fn application() -> impl View {
    gallery()
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct GalleryApp {
        mount: refract_web::Mount,
    }

    #[wasm_bindgen]
    impl GalleryApp {
        pub fn dispose(&self) {
            self.mount.dispose();
        }
        pub fn revision(&self) -> u64 {
            self.mount.revision()
        }
        pub fn dom_calls(&self) -> u64 {
            self.mount.dom_calls()
        }
        pub fn last_error(&self) -> Option<String> {
            self.mount.last_error()
        }
    }

    #[wasm_bindgen]
    pub fn start(host: web_sys::Element) -> Result<GalleryApp, JsValue> {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&info.to_string()))
        }));
        Ok(GalleryApp {
            mount: refract_web::mount(
                host,
                application(),
                refract_web::MountOptions {
                    sizing: refract_web::RootSizing::Fill,
                    ..Default::default()
                },
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract::{App, ElementKind, Patch};
    use refract_test::TestHost;

    #[test]
    fn changing_fit_preserves_images_and_their_sources() {
        let mut app = App::new(application());
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        let before: Vec<_> = host
            .elements
            .iter()
            .filter(|(_, e)| e.kind == ElementKind::Image)
            .map(|(id, e)| (*id, e.source.clone()))
            .collect();
        assert_eq!(before.len(), 3);
        host.press(&mut app, "Show full images").unwrap();
        let update = host.updates.last().unwrap();
        assert!(update
            .patches
            .iter()
            .all(|patch| matches!(patch, Patch::Update(_, _))));
        for (id, source) in before {
            let element = &host.elements[&id];
            assert_eq!(element.source, source);
            assert_eq!(element.style.image_fit, ImageFit::Contain);
        }
        host.press(&mut app, "Fill image frames").unwrap();
        assert!(host
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::Image)
            .all(|e| e.style.image_fit == ImageFit::Cover));
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser-tests"))]
mod fixture;

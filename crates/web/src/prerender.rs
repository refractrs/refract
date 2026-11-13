//! Evaluate the initial portable tree without committing effects or touching a browser.
use refract_core::{App, Element, ElementId, ElementKind, Patch, View, ViewError};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct PrerenderOptions {
    /// Emit a declarative open shadow root, matching default browser mounts.
    pub shadow: bool,
    /// Additional application CSS, included after Refract's base stylesheet.
    pub styles: String,
    /// Nonce for the generated stylesheet, if required by the embedding document.
    pub style_nonce: Option<String>,
}

impl Default for PrerenderOptions {
    fn default() -> Self {
        Self {
            shadow: true,
            styles: String::new(),
            style_nonce: None,
        }
    }
}

/// Render a fragment for insertion inside a host element. Effects are never started.
/// Use `hydrate` with the same initial view and shadow policy in the browser.
pub fn prerender(view: impl View, options: PrerenderOptions) -> Result<String, ViewError> {
    prerender_app(App::new(view), options)
}

/// Render with host inputs initialized in the same way as a browser mount.
/// The initializer must be browser-independent. Handles are kept alive through
/// rendering and then dropped; no effects are committed or state serialized.
/// Use identical initial values when hydrating the resulting HTML.
pub fn prerender_with_inputs<V: View, I>(
    options: PrerenderOptions,
    initialize: impl FnOnce(&mut refract_core::Runtime) -> (V, I),
) -> Result<String, ViewError> {
    let (app, inputs) = App::with_inputs(initialize);
    let result = prerender_app(app, options);
    drop(inputs);
    result
}

fn prerender_app(mut app: App, options: PrerenderOptions) -> Result<String, ViewError> {
    // Surface-dependent views can observe an unavailable build-time viewport.
    // Browser observations may change the initial tree, causing a clean remount.
    let _surface =
        refract_core::SurfaceDriver::install(&mut app, String::new(), None, Default::default());
    let pending = app
        .prepare()?
        .ok_or_else(|| ViewError::new("missing initial view"))?;
    let mut elements = BTreeMap::new();
    let mut children = BTreeMap::new();
    for patch in pending.patches() {
        match patch {
            Patch::Create(id, element) => {
                elements.insert(*id, element);
            }
            Patch::Children(parent, ids) => {
                children.insert(*parent, ids.as_slice());
            }
            _ => return Err(ViewError::new("unexpected patch in initial view")),
        }
    }
    fn render(
        parent: Option<ElementId>,
        elements: &BTreeMap<ElementId, &Element>,
        children: &BTreeMap<Option<ElementId>, &[ElementId]>,
        out: &mut String,
    ) -> Result<(), ViewError> {
        for id in children.get(&parent).copied().unwrap_or_default() {
            let e = elements[id];
            let tag = crate::html::tag(e);
            out.push('<');
            out.push_str(tag);
            for (key, value) in crate::html::attributes(*id, e) {
                out.push_str(&format!(" {key}=\"{}\"", crate::html::escape(&value)));
            }
            out.push('>');
            if matches!(tag, "img" | "input") {
                continue;
            }
            if e.kind == ElementKind::Text {
                out.push_str(&crate::html::escape(&e.text));
            } else if e.kind == ElementKind::TextInput {
                // HTML parsing strips one initial newline from textarea contents.
                if e.value.starts_with('\n') {
                    out.push('\n');
                }
                out.push_str(&crate::html::escape(&e.value));
            }
            if let Some(canvas) = &e.canvas {
                let fallback = canvas
                    .view_box
                    .map(|r| r.size())
                    .unwrap_or(refract_core::Size::new(300.0, 150.0));
                let size = refract_core::Size::new(
                    e.style.width.map_or(fallback.width, f64::from),
                    e.style.height.map_or(fallback.height, f64::from),
                );
                let commands = canvas.draw(size, std::time::Duration::ZERO)?;
                out.push_str(
                    &crate::svg::scene(&commands, size, &format!("refract-{}", id.0)).html(),
                );
            }
            render(Some(*id), elements, children, out)?;
            out.push_str(&format!("</{tag}>"));
        }
        Ok(())
    }
    let mut out = String::new();
    if options.shadow {
        out.push_str("<template shadowrootmode=\"open\" data-refract-prerender=\"1\">");
    }
    out.push_str("<style data-refract-style=\"1\"");
    if let Some(nonce) = options.style_nonce {
        out.push_str(&format!(" nonce=\"{}\"", crate::html::escape(&nonce)));
    }
    out.push('>');
    out.push_str(crate::html::BASE_STYLE);
    // CSS is application-authored, but must not terminate its HTML raw-text element.
    out.push_str(&options.styles.replace('<', "\\3c "));
    out.push_str("</style><div class=\"refract-root\" data-refract-prerender=\"1\">");
    render(None, &elements, &children, &mut out)?;
    out.push_str("</div>");
    if options.shadow {
        out.push_str("</template>");
    }
    Ok(out)
}

/// Versioned, browser-independent ABI: low 32 bits pointer, high 32 bits length;
/// the first byte is 0 for HTML or 1 for a UTF-8 error. The CLI drops the instance
/// after reading it, reclaiming all memory; no application-side runner is needed.
#[doc(hidden)]
pub fn encode_export(result: Result<String, ViewError>) -> u64 {
    let (status, text) = match result {
        Ok(html) => (0, html),
        Err(error) => (1, error.to_string()),
    };
    let mut bytes = Vec::with_capacity(text.len() + 1);
    bytes.push(status);
    bytes.extend_from_slice(text.as_bytes());
    let bytes = Box::leak(bytes.into_boxed_slice());
    ((bytes.len() as u64) << 32) | bytes.as_ptr() as usize as u64
}

/// Export an initial view from the app's existing WASM module for `build --prerender`.
/// The expression is evaluated only by the build-time renderer, not at browser startup.
///
/// ```ignore
/// refract_web::export_prerender!(application());
/// ```
#[macro_export]
macro_rules! export_prerender {
    ($view:expr) => {
        $crate::export_prerender!($view, $crate::PrerenderOptions::default());
    };
    ($view:expr, $options:expr) => {
        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn __refract_prerender_v1() -> u64 {
            $crate::encode_export($crate::prerender($view, $options))
        }
    };
}

/// Export a snapshot using the initializer shared with `hydrate_with_inputs`.
/// Initial values belong to the application; the snapshot ABI remains HTML-only.
#[macro_export]
macro_rules! export_prerender_with_inputs {
    ($initialize:expr) => {
        $crate::export_prerender_with_inputs!(
            $initialize,
            $crate::PrerenderOptions::default()
        );
    };
    ($initialize:expr, $options:expr) => {
        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn __refract_prerender_v1() -> u64 {
            $crate::encode_export($crate::prerender_with_inputs($options, $initialize))
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::prelude::*;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn host_inputs_are_deterministic_and_do_not_start_effects() {
        struct Inputs(Read<String>);
        let effects = Rc::new(Cell::new(0));
        let render = |initial: &str| {
            prerender_with_inputs(PrerenderOptions::default(), |rt| {
                let value = State::new(rt, initial.to_owned());
                let effects = effects.clone();
                let view = view_fn(move |cx| {
                    let inputs = cx.object::<Inputs>()?;
                    let effects = effects.clone();
                    cx.effect("host input effect", (), move |_| {
                        effects.set(effects.get() + 1);
                        Box::new(|| {})
                    });
                    Ok::<_, ViewError>(text(cx.read(&inputs.0)))
                })
                .environment_object(Rc::new(Inputs(value.read_only())));
                (view, value)
            })
            .unwrap()
        };
        let html = render("dark <initial>");
        assert!(html.contains("dark &lt;initial&gt;"));
        assert_eq!(html, render("dark <initial>"));
        assert_ne!(html, render("light"));
        assert_eq!(effects.get(), 0);
    }

    #[test]
    fn escapes_markup_and_does_not_start_effects() {
        let started = Rc::new(Cell::new(false));
        let flag = started.clone();
        let view = view_fn(move |cx| {
            let flag = flag.clone();
            cx.effect("must wait", (), move |_| {
                flag.set(true);
                Box::new(|| {})
            });
            Ok(text("<script>&\"'").label("\" onmouseover=\"evil"))
        });
        let html = prerender(
            view,
            PrerenderOptions {
                styles: "/* </style><script> */".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!started.get());
        assert!(html.contains("&lt;script&gt;&amp;&quot;&#39;"));
        assert!(html.contains("aria-label=\"&quot; onmouseover=&quot;evil\""));
        assert!(!html.contains("<script>"));
        assert_eq!(html.matches("</style>").count(), 1);
        assert!(html.starts_with("<template shadowrootmode=\"open\""));
    }

    #[test]
    fn renders_fields_images_and_light_dom() {
        let view = view_fn(|cx| {
            let value = cx.state("value", || "\n<&".to_owned());
            Ok(v_stack()
                .child(field(value).multiline(true).disabled(true))
                .child(image(ImageSource::embedded(
                    ImageFormat::Png,
                    vec![0, 1, 2],
                ))))
        });
        let html = prerender(
            view,
            PrerenderOptions {
                shadow: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!html.contains("<template"));
        assert!(html.contains("disabled=\"\""));
        assert!(html.contains(">\n\n&lt;&amp;</textarea>"));
        assert!(html.contains("data:image/png;base64,AAEC"));
    }

    #[test]
    fn overlay_alignment_is_included_in_the_initial_html() {
        let html = prerender(
            refract_core::z_stack()
                .alignment(Alignment::End, Alignment::Center)
                .child(text("Badge")),
            PrerenderOptions::default(),
        )
        .unwrap();
        assert!(html.contains("justify-items:end;"));
        assert!(html.contains("align-items:center;"));
    }

    #[test]
    fn radial_backgrounds_share_the_live_dom_css() {
        let html = prerender(
            view()
                .width(200.0)
                .height(100.0)
                .radius(12.0)
                .background_gradient(
                    RadialGradient::new(80.0, Color::BLUE, Color::WHITE).center(0.25, 0.75),
                ),
            PrerenderOptions::default(),
        )
        .unwrap();
        assert!(html.contains("background-image:radial-gradient(circle 80px at 25% 75% in srgb, rgba(45,87,232,1) 0%, rgba(255,255,255,1) 100%);"));
        assert!(html.contains("background-origin:border-box;"));
        assert!(html.contains("border-radius:12px 12px 12px 12px;"));
    }

    #[test]
    fn blur_filters_are_prerendered_and_invalid_amounts_are_rejected() {
        let html = prerender(
            text("glass").blur(3.0).backdrop_blur(16.0),
            PrerenderOptions::default(),
        )
        .unwrap();
        assert!(html.contains("filter:blur(3px);"));
        assert!(html.contains("backdrop-filter:blur(16px);"));
        assert!(prerender(text("invalid").blur(-1.0), PrerenderOptions::default()).is_err());
        assert!(prerender(
            text("invalid").backdrop_blur(f32::NAN),
            PrerenderOptions::default()
        )
        .is_err());
    }

    #[test]
    fn typography_is_prerendered_for_text_and_editable_fields() {
        let html = prerender(
            view_fn(|cx| {
                let value = cx.state("value", || "One two".to_owned());
                Ok(v_stack()
                    .child(
                        text("One two")
                            .italic()
                            .letter_spacing(-0.5)
                            .word_spacing(2.0),
                    )
                    .child(field(value).italic().letter_spacing(-0.5).word_spacing(2.0)))
            }),
            PrerenderOptions::default(),
        )
        .unwrap();
        for property in [
            "font-style:italic;",
            "letter-spacing:-0.5px;",
            "word-spacing:2px;",
        ] {
            assert_eq!(html.matches(property).count(), 2);
        }
    }
}

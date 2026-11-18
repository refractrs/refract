//! Selection ownership and observation checks shared by rich and plain fields.
#![cfg(target_arch = "wasm32")]

use refract_core::{prelude::*, TextHighlight, TextSelection, TextSpan};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::prelude::*;

#[derive(Clone)]
struct Handles {
    selection: State<Option<TextSelection>>,
    highlight: State<bool>,
    visible: State<bool>,
    formatted: State<bool>,
    wheel_scroll: Rc<Cell<Option<refract_core::ScrollMetrics>>>,
}

#[wasm_bindgen]
pub struct SelectionFixture(refract_web::InputMount<Handles>);

#[wasm_bindgen]
impl SelectionFixture {
    pub fn wheel_max_offset(&self) -> Option<f64> {
        self.0
            .inputs()
            .wheel_scroll
            .get()
            .map(|scroll| scroll.max_offset().x)
    }

    pub fn presentation(&self, highlight: bool, visible: bool) -> Result<(), JsValue> {
        self.0.update_inputs(move |handles, rt| {
            handles.highlight.set_if_changed(rt, highlight);
            handles.visible.set_if_changed(rt, visible);
        })
    }

    pub fn formatting(&self, enabled: bool) -> Result<(), JsValue> {
        self.0
            .update_inputs(move |handles, rt| handles.formatted.set_if_changed(rt, enabled))
    }

    pub fn select(&self, anchor: u32, focus: u32) -> Result<(), JsValue> {
        self.0.update_inputs(move |handles, rt| {
            handles
                .selection
                .set_if_changed(rt, Some(TextSelection { anchor, focus }));
        })
    }

    pub fn revision(&self) -> u64 {
        self.0.mount().revision()
    }
    pub fn last_error(&self) -> Option<String> {
        self.0.mount().last_error()
    }
    pub fn dispose(&self) {
        self.0.mount().dispose();
    }
}

#[wasm_bindgen]
pub fn start(
    host: web_sys::Element,
    rich: bool,
    shadow: bool,
) -> Result<SelectionFixture, JsValue> {
    refract_web::mount_with_inputs(
        host,
        refract_web::MountOptions {
            shadow,
            ..Default::default()
        },
        move |rt| {
            let value = State::new(rt, "ab😀cd".to_owned());
            let input_selection = State::new(rt, String::new());
            let handles = Handles {
                selection: State::new(rt, None),
                highlight: State::new(rt, false),
                visible: State::new(rt, true),
                formatted: State::new(rt, false),
                wheel_scroll: Rc::new(Cell::new(None)),
            };
            let inputs = handles.clone();
            let content = view_fn(move |cx| {
                let selection = cx.read_state(&inputs.selection);
                let highlight = cx.read_state(&inputs.highlight);
                let visible = cx.read_state(&inputs.visible);
                let formatted = cx.read_state(&inputs.formatted);
                let selected = inputs.selection.clone();
                let observed = input_selection.clone();
                let wheel_scroll = inputs.wheel_scroll.clone();
                let editor = field(value.binding())
                    .selection(inputs.selection.binding())
                    .selection_visible(visible)
                    .text_highlight(highlight.then_some(TextHighlight {
                        range: TextSelection {
                            anchor: 0,
                            focus: 2,
                        },
                        color: Color::rgb(255, 200, 0),
                    }))
                    .width(300.0)
                    .height(40.0)
                    .label("editor")
                    .on_wheel(move |cx, _| {
                        wheel_scroll.set(cx.geometry().and_then(|geometry| geometry.scroll))
                    })
                    .on_input(move |cx, _| {
                        let range = *selected.get(cx.runtime());
                        observed.set_if_changed(cx.runtime(), format!("{range:?}"));
                    });
                let editor = if rich {
                    editor.text_spans(if formatted {
                        vec![TextSpan {
                            start: 1,
                            end: 4,
                            bold: true,
                            italic: true,
                            underline: true,
                        }]
                    } else {
                        vec![]
                    })
                } else {
                    editor
                };
                Ok::<_, ViewError>(
                    v_stack()
                        .gap(8.0)
                        .child(editor)
                        .child(
                            text(
                                selection
                                    .map_or("none".into(), |s| format!("{}:{}", s.anchor, s.focus)),
                            )
                            .label("selection"),
                        )
                        .child(text(cx.read_state(&input_selection)).label("input-selection")),
                )
            });
            (content, handles)
        },
    )
    .map(SelectionFixture)
}

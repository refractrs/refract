//! Optional browser event conformance fixture.
use refract::prelude::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct EventsFixture {
    mount: refract_web::Mount,
    log: Rc<RefCell<Vec<String>>>,
    value: Binding<String>,
    disabled: Binding<bool>,
}
#[wasm_bindgen]
impl EventsFixture {
    pub fn take_events(&self) -> String {
        std::mem::take(&mut *self.log.borrow_mut()).join("|")
    }
    pub fn set_value(&self, value: String) -> Result<(), JsValue> {
        let binding = self.value.clone();
        self.mount
            .handle()
            .update(move |rt| binding.set_if_changed(rt, value))
    }
    pub fn set_disabled(&self, disabled: bool) -> Result<(), JsValue> {
        let binding = self.disabled.clone();
        self.mount
            .handle()
            .update(move |rt| binding.set_if_changed(rt, disabled))
    }
    pub fn last_error(&self) -> Option<String> {
        self.mount.last_error()
    }
    pub fn dispose(&self) {
        self.mount.dispose();
    }
}

#[wasm_bindgen]
pub fn start_events_fixture(host: web_sys::Element) -> Result<EventsFixture, JsValue> {
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let bindings = Rc::new(RefCell::new(None));
    let output = log.clone();
    let captured = bindings.clone();
    let content = view_fn(move |cx| {
        let value = cx.state("value", || "initial".to_owned());
        let disabled = cx.state("disabled", || false);
        *captured.borrow_mut() = Some((value.clone(), disabled.clone()));
        let parent = output.clone();
        let parent_hover = output.clone();
        let keys = output.clone();
        let second_keys = output.clone();
        let pointer = output.clone();
        let focus = output.clone();
        let hover = output.clone();
        let presses = output.clone();
        let input = output.clone();
        let current = value.clone();
        let submit = output.clone();
        let field_key = output.clone();
        Ok(v_stack()
            .label("events parent")
            .gap(12.0)
            .on_key(move |_, _| parent.borrow_mut().push("parent-key".into()))
            .on_hover(move |_, inside| {
                parent_hover
                    .borrow_mut()
                    .push(format!("parent-hover:{inside}"))
            })
            .child(
                view()
                    .label("event target")
                    .width(160.0)
                    .height(80.0)
                    .background(Color::BLUE)
                    .focusable(true)
                    .disabled(cx.read(&disabled))
                    .on_key(move |cx, event| {
                        keys.borrow_mut().push(format!(
                            "key:{:?}:{:?}:{}:{}",
                            event.key, event.state, event.repeat, event.is_composing
                        ));
                        if event.state == KeyState::Down
                            && !event.is_composing
                            && matches!(event.key, Key::Escape | Key::Enter)
                        {
                            cx.prevent_default();
                            cx.stop_propagation();
                        }
                    })
                    .on_key(move |_, _| second_keys.borrow_mut().push("second-key".into()))
                    .on_pointer(move |cx, event| {
                        pointer
                            .borrow_mut()
                            .push(format!("pointer:{:?}", event.phase));
                        if event.phase == PointerPhase::Down {
                            cx.capture_pointer();
                            cx.prevent_default();
                        }
                    })
                    .on_focus_change(move |_, focused| {
                        focus.borrow_mut().push(format!("focus:{focused}"))
                    })
                    .on_hover(move |_, inside| hover.borrow_mut().push(format!("hover:{inside}")))
                    .on_press(move |_| presses.borrow_mut().push("press".into()))
                    .child(text("Pointer target")),
            )
            .child(
                text("Keyboard target")
                    .label("keyboard target")
                    .focusable(true),
            )
            .child(
                field(value)
                    .label("event field")
                    .width(160.0)
                    .on_input(move |cx, text| {
                        input
                            .borrow_mut()
                            .push(format!("input:{text}:{}", current.get(cx.runtime())))
                    })
                    .on_submit(move |_| submit.borrow_mut().push("submit".into()))
                    .on_key(move |cx, event| {
                        field_key.borrow_mut().push("field-key".into());
                        if event.key == Key::ArrowDown {
                            cx.prevent_default();
                        }
                    }),
            ))
    });
    let mount = refract_web::mount(host, content, refract_web::MountOptions::default())?;
    let (value, disabled) = bindings.borrow_mut().take().unwrap();
    Ok(EventsFixture {
        mount,
        log,
        value,
        disabled,
    })
}

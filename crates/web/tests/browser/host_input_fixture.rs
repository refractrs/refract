//! Browser checks for host input initialization, hydration, and queued updates.
#![cfg(target_arch = "wasm32")]

use refract_core::prelude::*;
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::prelude::*;

struct Inputs {
    name: Read<String>,
    count: Read<u32>,
}

#[derive(Clone)]
struct Handles {
    name: State<String>,
    count: State<u32>,
    evaluations: Rc<Cell<u32>>,
}

fn initialize(rt: &mut Runtime) -> (impl View, Handles) {
    let name = State::new(rt, "initial".to_owned());
    let count = State::new(rt, 7);
    let evaluations = Rc::new(Cell::new(0));
    let observed = evaluations.clone();
    let content = v_stack()
        .child(view_fn(move |cx| {
            let inputs = cx.object::<Inputs>()?;
            observed.set(observed.get() + 1);
            Ok::<_, ViewError>(text(format!(
                "{}:{}", cx.read(&inputs.name), cx.read(&inputs.count)
            )).label("host inputs"))
        }))
        .child(view_fn(|cx| {
            let local = cx.state("local", || 0);
            let value = cx.read(&local);
            Ok::<_, ViewError>(text(format!("local:{value}"))
                .label("local state")
                .on_press(move |cx| local.update(cx.runtime(), |v| *v += 1)))
        }))
        .environment_object(Rc::new(Inputs {
            name: name.read_only(), count: count.read_only(),
        }));
    (content, Handles { name, count, evaluations })
}

#[wasm_bindgen]
pub fn snapshot() -> String {
    refract_web::prerender_with_inputs(Default::default(), initialize).unwrap()
}

#[wasm_bindgen]
pub struct InputFixture(refract_web::InputMount<Handles>);

#[wasm_bindgen]
impl InputFixture {
    pub fn update(&self, name: String, count: u32) -> Result<(), JsValue> {
        self.0.update_inputs(move |inputs, rt| {
            inputs.name.set_if_changed(rt, name);
            inputs.count.set_if_changed(rt, count);
        })
    }

    pub fn evaluations(&self) -> u32 {
        self.0.inputs().evaluations.get()
    }

    pub fn hydrated(&self) -> bool {
        self.0.mount().hydrated()
    }

    pub fn dispose(&self) {
        self.0.mount().dispose();
    }
}

#[wasm_bindgen]
pub fn start(host: web_sys::Element) -> Result<InputFixture, JsValue> {
    refract_web::hydrate_with_inputs(host, Default::default(), initialize).map(InputFixture)
}

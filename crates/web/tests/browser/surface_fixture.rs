//! Exercises the real browser bridge, including document ownership and rejected writes.
#![cfg(target_arch = "wasm32")]

use refract_core::*;
use refract_web::{Mount, MountOptions, RootSizing};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SurfaceFixture(Mount);

#[wasm_bindgen]
impl SurfaceFixture {
    pub fn dispose(&self) {
        self.0.dispose();
    }
    pub fn refresh_location(&self) {
        self.0.refresh_location();
    }
    pub fn last_error(&self) -> Option<String> {
        self.0.last_error()
    }
}

#[wasm_bindgen]
pub fn start_surface_fixture(
    host: web_sys::Element,
    managed: bool,
) -> Result<SurfaceFixture, JsValue> {
    let view = view_fn(|cx| {
        let surface = cx.object::<Surface>()?;
        let request = cx.state("request", || None::<Request>);
        let status = cx.read(&request).map(|request| cx.read(&request.status));
        let status = match status {
            None => "idle",
            Some(RequestStatus::Pending) => "pending",
            Some(RequestStatus::Applied) => "applied",
            Some(RequestStatus::Failed(_)) => "failed",
        };
        let mut content = v_stack()
            .child(text(cx.read(&surface.title)).label("title"))
            .child(
                text(
                    cx.read(&surface.location)
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                )
                .label("location"),
            )
            .child(
                text(
                    cx.read(&surface.viewport)
                        .map(|v| v.width.to_string())
                        .unwrap_or_default(),
                )
                .label("width"),
            )
            .child(text(status).label("status"));
        for (label, command) in [
            ("title", SurfaceCommand::SetTitle("Changed title".into())),
            (
                "push",
                SurfaceCommand::Navigate {
                    reference: "?step=two".into(),
                    mode: HistoryMode::Push,
                },
            ),
            (
                "replace",
                SurfaceCommand::Navigate {
                    reference: "?step=three".into(),
                    mode: HistoryMode::Replace,
                },
            ),
            (
                "reject",
                SurfaceCommand::Navigate {
                    reference: "https://invalid.invalid/".into(),
                    mode: HistoryMode::Push,
                },
            ),
        ] {
            let surface = surface.clone();
            let request = request.clone();
            content = content.child(text(label).label(format!("action-{label}")).on_press(
                move |cx| {
                    let result = match &command {
                        SurfaceCommand::SetTitle(title) => {
                            surface.set_title(cx.runtime(), title.clone())
                        }
                        SurfaceCommand::Navigate { reference, mode } => {
                            surface.navigate(cx.runtime(), reference.clone(), *mode)
                        }
                    };
                    if let Ok(result) = result {
                        request.set(cx.runtime(), Some(result));
                    }
                },
            ));
        }
        Ok(content)
    });
    refract_web::mount(
        host,
        view,
        MountOptions {
            sizing: RootSizing::Fill,
            manage_document: managed,
            ..Default::default()
        },
    )
    .map(SurfaceFixture)
}

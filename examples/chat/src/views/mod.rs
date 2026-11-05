//! Views are split by responsibility. Layout components do not read message
//! contents; leaf consumers establish the graph dependencies they actually need.
mod composer;
mod conversation;
mod message;
mod sidebar;

use crate::model::ChatModel;
use refract::prelude::*;
use std::rc::Rc;

pub const BACKGROUND: Color = Color::rgb(250, 249, 246);
pub(super) const SIDE: Color = Color::rgb(239, 238, 232);
pub(super) const INK: Color = Color::rgb(43, 48, 42);
pub(super) const MUTED: Color = Color::rgb(119, 122, 111);
pub(super) const LINE: Color = Color::rgb(222, 223, 214);
pub(super) const GREEN: Color = Color::rgb(65, 86, 60);
pub(super) const PALE: Color = Color::rgb(226, 231, 218);
pub(super) const SERIF: &str = "Georgia, serif";
pub(super) const MONO: &str = "'SFMono-Regular', Consolas, monospace";

#[view]
pub(super) fn shell(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    let open = cx.read_state(&model.sidebar_open);
    h_stack()
        .child(if open {
            sidebar::sidebar().into_any()
        } else {
            Empty.into_any()
        })
        .child(
            v_stack()
                .child(header())
                .child(
                    conversation::selected()
                        .grow(1.0)
                        .min_height(0.0)
                        .fill_width(),
                )
                .grow(1.0)
                .min_width(0.0)
                .fill_height(),
        )
        .fill_width()
        .fill_height()
        .background(BACKGROUND)
        .foreground(INK)
}

fn header() -> impl View {
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::SpaceBetween)
        .child(
            h_stack()
                .gap(14.0)
                .alignment(Alignment::Center)
                .child(toggle_sidebar())
                .child(text("Refract Chat").font_size(14.0).font_weight(600)),
        )
        .child(
            text("●  LOCAL DEMO")
                .font_family(MONO)
                .font_size(10.0)
                .foreground(GREEN)
                .padding(Insets::symmetric(11.0, 7.0))
                .background(PALE)
                .radius(20.0),
        )
        .padding(Insets::symmetric(28.0, 18.0))
        .fill_width()
}

#[view]
fn toggle_sidebar(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    #[action(model)]
    let toggle = |action| {
        model
            .sidebar_open
            .update(action.runtime(), |open| *open = !*open)
    };
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::Center)
        .child(text("☰").font_size(19.0))
        .on_press(toggle)
        .label("Toggle sidebar")
        .width(30.0)
        .height(30.0)
        .background(BACKGROUND)
        .foreground(MUTED)
        .radius(6.0)
}

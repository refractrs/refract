//! Views are split by responsibility. Layout components do not read message
//! contents; leaf consumers establish the graph dependencies they actually need.
mod composer;
mod conversation;
mod message;
mod sidebar;

use crate::model::ChatModel;
use refract::prelude::*;
use std::rc::Rc;

// Light-theme tokens from www/src/app/globals.css.
pub const BACKGROUND: Color = Color::rgb(251, 252, 254);
pub(super) const SIDE: Color = Color::rgb(241, 245, 251);
pub(super) const INK: Color = Color::rgb(24, 40, 63);
pub(super) const MUTED: Color = Color::rgb(82, 98, 123);
pub(super) const LINE: Color = Color::rgb(203, 213, 227);
pub(super) const ACCENT: Color = Color::rgb(40, 87, 183);
pub(super) const PALE: Color = Color::rgb(220, 232, 251);
pub(super) const SANS: &str = "Inter, 'Helvetica Neue', Arial, sans-serif";
pub(super) const MONO: &str = "'SFMono-Regular', Consolas, monospace";

fn avatar_gradient(radius: f32) -> RadialGradient {
    RadialGradient::new(radius, Color(255, 255, 255, 220), Color(117, 159, 224, 36))
        .center(0.25, 0.2)
}

#[view]
pub(super) fn shell(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    let open = cx.read_derived(&model.sidebar_visible);
    let compact = cx.read_derived(&model.compact);
    h_stack()
        .child(if open && !compact {
            sidebar::sidebar(false).into_any()
        } else {
            Empty.into_any()
        })
        .child(
            v_stack()
                .child(header())
                .child(crate::surface::navigation_status())
                .child(if compact && open {
                    sidebar::sidebar(true).grow(1.0).min_height(0.0).into_any()
                } else {
                    conversation::selected()
                        .grow(1.0)
                        .min_height(0.0)
                        .fill_width()
                        .into_any()
                })
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
                .child(text("Chat").font_size(14.0).font_weight(600)),
        )
        .child(
            text("●  LOCAL DEMO")
                .font_family(MONO)
                .font_size(10.0)
                .foreground(ACCENT)
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
    let toggle = |action| model.toggle_sidebar(action.runtime());
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

//! Views are split by responsibility. Layout components do not read message
//! contents; leaf consumers establish the graph dependencies they actually need.
mod composer;
mod conversation;
mod message;
mod sidebar;

use crate::model::ChatModel;
use refract::prelude::*;
use std::{rc::Rc, time::Duration};

fn color_motion() -> Animation {
    Animation::ease_out(Duration::from_millis(140))
}
fn entrance_motion() -> Animation {
    Animation::ease_out(Duration::from_millis(180))
}
fn sidebar_motion() -> Animation {
    Animation::ease_in_out(Duration::from_millis(220))
}

const SIDEBAR_WIDTH: f32 = 262.0;

pub(super) const SANS: &str = "Inter, 'Helvetica Neue', Arial, sans-serif";
pub(super) const MONO: &str = "'SFMono-Regular', Consolas, monospace";

#[view]
pub(super) fn shell(cx: &mut ViewContext<'_>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let model: Rc<ChatModel>;
    let open = cx.read_derived(&model.sidebar_visible);
    let compact = cx.read_derived(&model.compact);
    h_stack()
        .child(
            view()
                .child(if open && !compact {
                    sidebar::sidebar(false)
                        .fill_height()
                        .transition(Transition::offset(-SIDEBAR_WIDTH, 0.0))
                        .into_any()
                } else {
                    Empty.into_any()
                })
                .width(if open && !compact { SIDEBAR_WIDTH } else { 0.0 })
                .fill_height()
                .clipped()
                .animation(sidebar_motion(), open),
        )
        .child(
            v_stack()
                .child(header())
                .child(crate::surface::navigation_status())
                .child(if compact && open {
                    sidebar::sidebar(true)
                        .grow(1.0)
                        .min_height(0.0)
                        .transition(
                            Transition::opacity().combined_with(Transition::offset(-16.0, 0.0)),
                        )
                        .key("compact-sidebar")
                        .into_any()
                } else {
                    conversation::selected()
                        .grow(1.0)
                        .min_height(0.0)
                        .fill_width()
                        .key("conversation-pane")
                        .into_any()
                })
                .grow(1.0)
                .min_width(0.0)
                .fill_height()
                .animation(sidebar_motion(), open),
        )
        .fill_width()
        .fill_height()
        .background(palette.background)
        .foreground(palette.ink)
}

#[view]
fn header(cx: &mut ViewContext<'_>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let model: Rc<ChatModel>;
    let selected = cx.read_state(&model.selected);
    let chat = cx
        .read_state(&model.conversations)
        .into_iter()
        .find(|chat| chat.id == selected)
        .expect("model keeps a selected conversation");
    let title = cx.read_state(&chat.title);
    #[action(model, chat)]
    let delete = |action| model.delete(action.runtime(), chat.id);
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::SpaceBetween)
        .gap(12.0)
        .child(
            h_stack()
                .gap(14.0)
                .alignment(Alignment::Center)
                .child(toggle_sidebar())
                .child(
                    text(title)
                        .font_size(14.0)
                        .font_weight(600)
                        .truncate()
                        .grow(1.0)
                        .min_width(0.0),
                )
                .grow(1.0)
                .min_width(0.0),
        )
        .child(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(text("Delete Chat").font_size(11.0).padding(6.0))
                .on_press(delete)
                .label("Delete Chat")
                .background(palette.background)
                .foreground(palette.muted)
                .radius(5.0),
        )
        .padding(Insets::symmetric(28.0, 18.0))
        .fill_width()
}

#[view]
fn toggle_sidebar(cx: &mut ViewContext<'_>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let model: Rc<ChatModel>;
    #[action(model)]
    let toggle = |action| model.toggle_sidebar(action.runtime());
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::Center)
        .child(
            h_stack()
                .child(
                    view()
                        .width(5.0)
                        .fill_height()
                        .border(Border::default().right(1.5, palette.muted)),
                )
                .width(18.0)
                .height(16.0)
                .border(Border::all(1.5, palette.muted))
                .radius(3.0),
        )
        .on_press(toggle)
        .label("Toggle sidebar")
        .width(30.0)
        .height(30.0)
        .background(palette.background)
        .foreground(palette.muted)
        .radius(6.0)
}

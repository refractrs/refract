use super::*;
use crate::model::{Delivery, Message, Role};

pub(super) fn message(message: Rc<Message>) -> impl View {
    let user = message.role == Role::User;
    v_stack()
        .gap(10.0)
        .child(
            h_stack()
                .gap(9.0)
                .alignment(Alignment::Center)
                .child(avatar(user))
                .child(
                    text(if user { "You" } else { "Chat" })
                        .font_size(12.0)
                        .font_weight(600),
                ),
        )
        .child(message_body(message.clone()))
        .child(message_status(message))
}

fn avatar(user: bool) -> impl View {
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::Center)
        .child(
            text(if user { "Y" } else { "c" })
                .font_size(11.0)
                .font_weight(400),
        )
        .width(28.0)
        .height(28.0)
        .background(if user { SIDE } else { PALE })
        .background_gradient(avatar_gradient(28.0))
        .foreground(ACCENT)
        .radius(8.0)
}

#[view]
fn message_body(cx: &mut ViewContext<'_>, message: Rc<Message>) -> impl View {
    // The only view subscribed to streaming text. No list or parent reads body.
    let body = cx.read_state(&message.body);
    text(if body.is_empty() {
        match cx.read_state(&message.delivery) {
            Delivery::Streaming => "Thinking…",
            Delivery::Stopped => "Response stopped before any text arrived.",
            Delivery::Failed => "The reply could not be completed.",
            Delivery::Complete => "No response text.",
        }
        .into()
    } else {
        body
    })
    .font_size(15.0)
    .line_height(25.0)
    .foreground(INK)
}

#[view]
fn message_status(cx: &mut ViewContext<'_>, message: Rc<Message>) -> impl View {
    let delivery = cx.read_state(&message.delivery);
    if message.role == Role::User {
        Empty.into_any()
    } else {
        text(match delivery {
            Delivery::Streaming => "●  Writing…",
            Delivery::Complete => "LOCAL RESPONSE",
            Delivery::Stopped => "RESPONSE STOPPED",
            Delivery::Failed => "RESPONSE FAILED",
        })
        .font_family(MONO)
        .font_size(9.0)
        .foreground(MUTED)
        .into_any()
    }
}

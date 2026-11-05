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
                .child(
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(
                            text(if user { "Y" } else { "r" })
                                .font_family(if user { "system-ui" } else { SERIF })
                                .font_size(if user { 11.0 } else { 22.0 })
                                .line_height(if user { 11.0 } else { 22.0 })
                                // Center the lowercase glyph optically, not just its line box.
                                .padding(Insets {
                                    bottom: if user { 0.0 } else { 4.0 },
                                    ..Default::default()
                                }),
                        )
                        .width(28.0)
                        .height(28.0)
                        .background(if user { SIDE } else { PALE })
                        .foreground(GREEN)
                        .radius(8.0),
                )
                .child(
                    text(if user { "You" } else { "Refract" })
                        .font_size(12.0)
                        .font_weight(600),
                ),
        )
        .child(message_body(message.clone()))
        .child(message_status(message))
}

#[view]
fn message_body(cx: &mut ViewContext<'_>, message: Rc<Message>) -> impl View {
    // The only view subscribed to streaming text. No list or parent reads body.
    let body = cx.read_state(&message.body);
    text(if body.is_empty() {
        match cx.read_state(&message.delivery) {
            Delivery::Streaming => "Thinking…",
            Delivery::Stopped => "Response stopped before any text arrived.",
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
        })
        .font_family(MONO)
        .font_size(9.0)
        .foreground(MUTED)
        .into_any()
    }
}

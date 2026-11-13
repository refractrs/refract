use super::*;
use crate::model::Conversation;

#[view]
pub(super) fn composer(cx: &mut ViewContext<'_>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let chat: Rc<Conversation>;
    #[environment_object]
    let model: Rc<ChatModel>;
    let within = cx.state("focus-within", || false);
    let active = cx.read(&within);
    #[action(chat)]
    let submit = |action| chat.send(action.runtime());
    v_stack()
        .gap(12.0)
        .alignment(Alignment::Center)
        .child(
            v_stack()
                .gap(8.0)
                .child(
                    field(chat.draft.binding())
                        .focused_value(
                            model.focus.binding(),
                            crate::model::ChatFocus::Composer(chat.id),
                        )
                        .multiline(true)
                        .label("Message Refract")
                        .placeholder("What's on your mind?")
                        .on_submit(submit)
                        .style(refract::Style {
                            height: Some(64.0),
                            fill_width: true,
                            font_size: Some(14.0),
                            line_height: Some(23.0),
                            background: Some(Color(0, 0, 0, 0)),
                            foreground: Some(palette.ink),
                            border: Some(Border::all(0.0, Color::WHITE)),
                            focus_ring: Some(FocusRing::none()),
                            ..Default::default()
                        }),
                )
                .child(
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::SpaceBetween)
                        .child(
                            text("Local assistant")
                                .font_size(11.0)
                                .foreground(palette.muted),
                        )
                        .child(send_control()),
                )
                .padding(16.0)
                .background(if active {
                    palette.focused_card
                } else {
                    palette.card
                })
                .on_focus_within_change(move |cx, active| {
                    within.set_if_changed(cx.runtime(), active);
                })
                .border(Border::all(1.0, palette.line))
                .radius(14.0)
                .focus_within(FocusRing::new(palette.accent, 2.0, 2.0))
                .shadow(BoxShadow::new(0.0, 4.0, 20.0, palette.shadow))
                .max_width(740.0)
                .fill_width()
                .animation(color_motion(), active),
        )
        .child(
            text("Scripted replies · Chats stay in this session · ⌘ / Ctrl + Enter to send")
                .font_size(10.0)
                .foreground(palette.muted)
                .text_align(Alignment::Center),
        )
        .padding(Insets::symmetric(34.0, 18.0))
        .fill_width()
}

#[view]
fn send_control(cx: &mut ViewContext<'_>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let chat: Rc<Conversation>;
    let busy = cx.read_state(&chat.request).is_some();
    let enabled = cx.read_derived(&chat.can_send);
    #[action(chat)]
    let send_or_stop = |action| {
        if busy {
            chat.stop(action.runtime());
        } else {
            chat.send(action.runtime());
        }
    };
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::Center)
        .child(
            text(if busy { "■  Stop" } else { "Send  ↑" })
                .font_size(12.0)
                .font_weight(600)
                .padding(Insets::symmetric(15.0, 9.0)),
        )
        .on_press(send_or_stop)
        .label(if busy {
            "Stop response"
        } else {
            "Send message"
        })
        .disabled(!busy && !enabled)
        .background(if busy || enabled {
            palette.accent
        } else {
            palette.side
        })
        .foreground(if busy || enabled {
            palette.on_accent
        } else {
            palette.muted
        })
        .radius(8.0)
        .animation(color_motion(), (busy, enabled))
}

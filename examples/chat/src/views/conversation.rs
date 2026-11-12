use super::*;
use crate::model::Conversation;

#[view]
pub(super) fn selected(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    let selected = cx.read_state(&model.selected);
    let chats = cx.read_state(&model.conversations);
    let chat = chats
        .into_iter()
        .find(|c| c.id == selected)
        .expect("model keeps a selected conversation");
    // A key changes the pane's identity on selection. Drafts live in the model,
    // while host-owned selection/scroll belongs to the mounted conversation.
    pane()
        .transition(Transition::asymmetric(Transition::opacity(), Transition::identity()))
        .environment_object(chat)
        .key(selected.0.to_string())
        .animation(Animation::ease_out(Duration::from_millis(120)), selected)
}

fn pane() -> impl View {
    v_stack()
        .child(title_bar())
        .child(
            transcript()
                .scrollable()
                .initial_position(ScrollPosition::End)
                .follow(ScrollFollow::EndWhenPinned)
                .grow(1.0)
                .min_height(0.0)
                .fill_width(),
        )
        .child(composer::composer())
        .fill_width()
        .fill_height()
        .min_height(0.0)
}

#[view]
fn title_bar(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    #[environment_object]
    let chat: Rc<Conversation>;
    let title = cx.read_state(&chat.title);
    #[action(model, chat)]
    let delete = |action| model.delete(action.runtime(), chat.id);
    h_stack()
        .justify(Alignment::SpaceBetween)
        .alignment(Alignment::Center)
        .gap(12.0)
        .child(text(title).font_size(12.0).foreground(MUTED).grow(1.0))
        .child(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(text("Delete chat").font_size(11.0).padding(6.0))
                .on_press(delete)
                .label("Delete chat")
                .background(BACKGROUND)
                .foreground(MUTED)
                .radius(5.0),
        )
        .padding(Insets::symmetric(28.0, 4.0))
        .fill_width()
}

#[view]
fn transcript(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let chat: Rc<Conversation>;
    let messages = cx.read_state(&chat.messages);
    let message_count = messages.len();
    let contents = if messages.is_empty() {
        welcome().into_any()
    } else {
        v_stack()
            .gap(30.0)
            .children(messages.into_iter().map(|message| {
                let key = message.id.0.to_string();
                message::message(message)
                    .transition(Transition::asymmetric(
                        Transition::opacity().combined_with(Transition::offset(0.0, 4.0)),
                        Transition::identity(),
                    ))
                    .key(key)
            }))
            .into_any()
    };
    v_stack()
        .alignment(Alignment::Center)
        .child(
            contents
                // Size the outer padding wrapper, so the column never sizes
                // itself to the current message text while a reply streams.
                .padding(Insets::symmetric(34.0, 32.0))
                .max_width(740.0)
                .fill_width(),
        )
        .fill_width()
        .animation(entrance_motion(), message_count)
}

#[view]
fn welcome(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let chat: Rc<Conversation>;
    v_stack()
        .gap(24.0)
        .child(
            text("A fresh page.\nWhere shall we begin?")
                .font_family(SANS)
                .font_size(38.0)
                .line_height(48.0)
                .foreground(INK),
        )
        .child(
            text("Explore an idea, untangle a problem, or make a little room to think.")
                .font_size(14.0)
                .line_height(23.0)
                .foreground(MUTED),
        )
        .child(
            v_stack().gap(10.0).children(
                [
                    (
                        "Think clearly",
                        "How can I design a more focused workspace?",
                    ),
                    (
                        "Build something",
                        "Explain the Rust state and dataflow in Refract.",
                    ),
                    (
                        "Find a change of scene",
                        "Help me plan a slow weekend away.",
                    ),
                ]
                .into_iter()
                .map(|(title, prompt)| {
                    let chat = chat.clone();
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(
                            v_stack()
                                .gap(5.0)
                                .child(text(title).font_size(13.0).font_weight(600))
                                .child(text(prompt).font_size(12.0).foreground(MUTED))
                                .padding(16.0)
                                .fill_width(),
                        )
                        .on_press(move |action| chat.suggest(action.runtime(), prompt))
                        .label(title)
                        .fill_width()
                        .background(BACKGROUND)
                        .border(Border::all(1.0, LINE))
                        .radius(10.0)
                }),
            ),
        )
        .fill_width()
}

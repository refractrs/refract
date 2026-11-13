use super::*;
use crate::model::Conversation;

#[view]
pub(super) fn sidebar(cx: &mut ViewContext<'_>, compact: bool) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let model: Rc<ChatModel>;
    #[action(model)]
    let create = |action| model.new_chat(action.runtime());
    let content = v_stack()
        .gap(24.0)
        .child(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(
                    h_stack()
                        .gap(12.0)
                        .alignment(Alignment::Center)
                        .child(text("+").font_size(21.0))
                        .child(text("New Conversation").font_size(13.0)),
                )
                .on_press(create)
                .label("New Conversation")
                .fill_width()
                .height(44.0)
                .background(palette.accent)
                .background_gradient(LinearGradient::new(
                    160.0,
                    palette.primary_start,
                    palette.primary_end,
                ))
                .foreground(palette.on_accent)
                .radius(9.0),
        )
        .child(
            field(model.query.binding())
                .focused_value(model.focus.binding(), crate::model::ChatFocus::Search)
                .label("Search conversations")
                .placeholder("Search conversations…")
                .style(refract::Style {
                    font_size: Some(12.0),
                    fill_width: true,
                    height: Some(36.0),
                    padding: Insets::symmetric(10.0, 6.0),
                    background: Some(palette.side),
                    foreground: Some(palette.ink),
                    border: Some(Border::all(1.0, palette.line)),
                    radius: 7.0.into(),
                    ..Default::default()
                }),
        )
        .child(
            v_stack()
                .gap(13.0)
                .child(
                    text("YOUR CONVERSATIONS")
                        .font_family(MONO)
                        .font_size(9.0)
                        .foreground(palette.muted),
                )
                .child(conversation_list())
                .padding(6.0)
                .scrollable()
                .grow(1.0)
                .min_height(0.0),
        )
        .child(
            v_stack().gap(16.0).child(
                h_stack()
                    .gap(10.0)
                    .alignment(Alignment::Center)
                    .child(
                        h_stack()
                            .alignment(Alignment::Center)
                            .justify(Alignment::Center)
                            .child(text("Y").font_size(12.0).line_height(12.0))
                            .width(40.0)
                            .height(40.0)
                            .foreground(palette.accent)
                            .background(palette.pale)
                            .background_gradient(palette.avatar_gradient(40.0))
                            .radius(20.0),
                    )
                    .child(
                        v_stack()
                            .gap(3.0)
                            .child(text("Your workspace").font_size(12.0).font_weight(600))
                            .child(
                                text("On this device · this session")
                                    .font_size(10.0)
                                    .foreground(palette.muted),
                            ),
                    ),
            ),
        )
        .fill_height()
        .padding(Insets::symmetric(18.0, 24.0))
        .fill_height()
        .background(palette.side);
    if compact {
        content.fill_width()
    } else {
        content.width(SIDEBAR_WIDTH)
    }
}

#[view]
fn conversation_list(cx: &mut ViewContext<'_>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let model: Rc<ChatModel>;
    let chats = cx.read_state(&model.conversations);
    let query = cx.read_state(&model.query).to_lowercase();
    let matches: Vec<_> = chats
        .into_iter()
        .filter(|chat| cx.read_state(&chat.title).to_lowercase().contains(&query))
        .collect();
    if matches.is_empty() {
        text("No conversations found.")
            .font_size(12.0)
            .foreground(palette.muted)
            .padding(8.0)
            .into_any()
    } else {
        v_stack()
            .gap(6.0)
            .children(matches.into_iter().map(|chat| {
                let key = chat.id.0.to_string();
                conversation_row(chat).key(key)
            }))
            .into_any()
    }
}

#[view]
fn conversation_row(cx: &mut ViewContext<'_>, chat: Rc<Conversation>) -> impl View {
    let palette = crate::theme::palette(cx)?;
    #[environment_object]
    let model: Rc<ChatModel>;
    let selected = cx.read_state(&model.selected) == chat.id;
    let title = cx.read_state(&chat.title);
    let active = cx.read_state(&chat.request).is_some();
    #[action(model, chat)]
    let select = |action| model.select(action.runtime(), chat.id);
    h_stack()
        .alignment(Alignment::Center)
        .justify(Alignment::Center)
        .child(
            h_stack()
                .gap(8.0)
                .alignment(Alignment::Center)
                .child(
                    text(if active { "●" } else { "○" })
                        .font_size(13.0)
                        .foreground(if active {
                            palette.accent
                        } else {
                            palette.muted
                        }),
                )
                .child(text(&title).font_size(12.0).line_height(19.0).grow(1.0))
                .padding(10.0)
                .fill_width(),
        )
        .on_press(select)
        .label(format!("Open {title}"))
        .fill_width()
        .background(if selected { palette.pale } else { palette.side })
        .radius(7.0)
        // Clear the old highlight immediately, including when insertion moves this row.
        .animation(selected.then(color_motion), selected)
}

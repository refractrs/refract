use refract_core::{prelude::*, App, ElementKind, Event, Field, Stack, Text};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc};

#[test]
fn typography_preserves_text_and_field_builders_and_rejects_invalid_updates() {
    let make_view = |spacing| {
        view_fn(move |cx| {
            let value = cx.state("value", || "One two".to_owned());
            let title: Text = text("One two")
                .italic()
                .letter_spacing(spacing)
                .word_spacing(2.0)
                .heading(2);
            let input: Field = field(value)
                .italic()
                .letter_spacing(spacing)
                .word_spacing(2.0)
                .placeholder("Type here");
            Ok(v_stack().child(title).child(input))
        })
    };
    let mut app = App::new(make_view(-0.5));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    let children = host.children[&Some(root)].clone();
    assert_eq!(host.elements.len(), 3, "typography adds no wrapper");
    for id in &children {
        let style = &host.elements[id].style;
        assert_eq!(style.italic, Some(true));
        assert_eq!(style.letter_spacing, Some(-0.5));
        assert_eq!(style.word_spacing, Some(2.0));
    }
    assert!(app.dispatch(children[1], Event::Input("Edited".into())));
    host.flush(&mut app).unwrap();
    app.set_view(make_view(1.0));
    host.flush(&mut app).unwrap();
    assert_eq!(host.children[&Some(root)], children);
    assert_eq!(host.elements[&children[1]].value, "Edited");
    app.set_view(make_view(f32::NAN));
    assert!(app.prepare().is_err());
    assert_eq!(host.elements[&children[0]].style.letter_spacing, Some(1.0));
}

#[test]
fn gradient_modifiers_accept_both_kinds_on_builders_and_composed_views() {
    let radial = RadialGradient::new(80.0, Color::BLUE, Color::WHITE);
    let linear = LinearGradient::new(90.0, Color::BLUE, Color::WHITE);
    let container: Container = view().background_gradient(radial.clone());
    let mut app = App::new(
        container.child(view_fn(|_| Ok(text("Content"))).background_gradient(linear.clone())),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    let child = host.children[&Some(root)][0];
    assert_eq!(
        host.elements[&root].style.background_gradient,
        Some(radial.into())
    );
    assert_eq!(
        host.elements[&child].style.background_gradient,
        Some(linear.into())
    );
}

#[test]
fn field_modifiers_preserve_the_builder_and_style_the_editable_element() {
    for disabled in [false, true] {
        let submissions = Rc::new(RefCell::new(Vec::new()));
        let received = submissions.clone();
        let mut app = App::new(view_fn(move |cx| {
            let value = cx.state("value", || "initial".to_owned());
            let submitted = value.clone();
            let received = received.clone();
            let editor: Field = field(value)
                .width(280.0)
                .label("Notes")
                .height(160.0)
                .multiline(true)
                .background(Color::WHITE)
                .placeholder("Write a note")
                .font_size(18.0)
                .disabled(disabled)
                .on_submit(move |cx| {
                    received.borrow_mut().push(submitted.get(cx.runtime()));
                });
            Ok(editor.padding(12.0).background(Color::BLUE))
        }));
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        assert_eq!(host.elements.len(), 2, "only padding adds a box");
        let wrapper = host.children[&None][0];
        let id = host.children[&Some(wrapper)][0];
        let element = &host.elements[&id];
        assert_eq!(element.kind, ElementKind::TextInput);
        assert!(element.multiline);
        assert_eq!(element.label.as_deref(), Some("Notes"));
        assert_eq!(element.placeholder.as_deref(), Some("Write a note"));
        assert_eq!(element.disabled, disabled);
        assert_eq!(element.style.width, Some(280.0));
        assert_eq!(element.style.height, Some(160.0));
        assert_eq!(element.style.font_size, Some(18.0));
        assert_eq!(element.style.background, Some(Color::WHITE));
        assert_eq!(host.elements[&wrapper].style.background, Some(Color::BLUE));
        assert_eq!(host.elements[&wrapper].style.width, None);
        assert!(!host.elements[&wrapper].disabled);

        assert_eq!(app.dispatch(id, Event::Input("edited".into())), !disabled);
        host.flush(&mut app).unwrap();
        assert_eq!(host.children[&Some(wrapper)], [id]);
        assert_eq!(
            host.elements[&id].value,
            if disabled { "initial" } else { "edited" }
        );
        assert_eq!(app.dispatch(id, Event::Submit), !disabled);
        assert_eq!(
            *submissions.borrow(),
            if disabled { vec![] } else { vec!["edited"] }
        );
    }
}

#[test]
fn styled_builders_keep_child_layout_and_semantic_methods() {
    let title: Text = text("Title").font_size(24.0).heading(2);
    let stack: Stack = v_stack()
        .width(300.0)
        .child(title)
        .background(Color::WHITE)
        .gap(8.0)
        .height(100.0)
        .child(
            image("/cover.png")
                .fit(ImageFit::Cover)
                .label("Cover"),
        );
    let mut app = App::new(stack);
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.elements.len(), 3);
    let root = host.children[&None][0];
    assert_eq!(host.elements[&root].style.width, Some(300.0));
    assert_eq!(host.elements[&root].style.gap, 8.0);
    let children = &host.children[&Some(root)];
    assert_eq!(host.elements[&children[0]].role.as_deref(), Some("heading"));
    assert_eq!(host.elements[&children[1]].label.as_deref(), Some("Cover"));
}

#[test]
fn styled_scroll_views_keep_behavior_and_both_handler_orders() {
    for press_first in [false, true] {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let press = calls.clone();
        let scroll = calls.clone();
        let on_press = move |_: &mut ActionContext<'_>| press.borrow_mut().push("press");
        let on_scroll =
            move |_: &mut ActionContext<'_>, _: &ScrollEvent| scroll.borrow_mut().push("scroll");
        let container: ScrollView = scroll_view(text("content"))
            .height(100.0)
            .follow(ScrollFollow::EndWhenPinned)
            .width(200.0)
            .initial_position(ScrollPosition::End)
            .label("History");
        let container: ScrollView = if press_first {
            container.on_press(on_press).on_scroll(on_scroll)
        } else {
            container.on_scroll(on_scroll).on_press(on_press)
        };
        let mut app = App::new(container);
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        assert_eq!(host.elements.len(), 2);
        let id = host.children[&None][0];
        let element = &host.elements[&id];
        assert_eq!(element.style.height, Some(100.0));
        let options = element.scroll.unwrap();
        assert_eq!(options.follow, ScrollFollow::EndWhenPinned);
        assert_eq!(options.initial_position, ScrollPosition::End);
        assert!(options.observe);
        assert!(app.dispatch(id, Event::Press));
        assert!(app.dispatch(
            id,
            Event::Scroll(ScrollEvent {
                metrics: ScrollMetrics::default(),
                delta: ScrollOffset::default(),
            })
        ));
        assert_eq!(*calls.borrow(), ["press", "scroll"]);
    }
}

#[test]
fn composed_views_and_structural_modifiers_keep_their_wrappers() {
    struct Content;
    impl View for Content {
        fn body(&self, _: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
            Ok(text("Content"))
        }
    }
    let mut app = App::new(Content.width(100.0).padding(12.0).frame(200.0, 80.0));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.elements.len(), 4);
    let frame = host.children[&None][0];
    let padding = host.children[&Some(frame)][0];
    let sized = host.children[&Some(padding)][0];
    let text = host.children[&Some(sized)][0];
    assert_eq!(host.elements[&frame].style.width, Some(200.0));
    assert_eq!(host.elements[&padding].style.padding, Insets::from(12.0));
    assert_eq!(host.elements[&sized].style.width, Some(100.0));
    assert_eq!(host.elements[&text].kind, ElementKind::Text);
    assert_eq!(host.elements[&text].style.width, None);
}

#[test]
fn typed_containers_compose_mixed_views_without_extra_host_nodes() {
    fn content() -> impl View {
        let cover: Image = image("/cover.png").width(120.0).fit(ImageFit::Cover);
        let overlay: Overlay = z_stack()
            .width(200.0)
            .children([cover.into_any(), text("Caption").font_size(16.0).into_any()]);
        let flexible: Spacer = spacer().grow(2.0);
        let row: Stack = h_stack().gap(8.0).child(overlay).child(flexible);
        let container: Container = view().width(300.0).child(row).maybe_child(None::<Text>);
        container
    }
    let mut app = App::new(content());
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.elements.len(), 6);
    let root = host.children[&None][0];
    let row = host.children[&Some(root)][0];
    let row_children = &host.children[&Some(row)];
    let overlay = row_children[0];
    assert_eq!(host.elements[&row].style.layout, refract_core::Layout::Row);
    assert_eq!(
        host.elements[&overlay].style.layout,
        refract_core::Layout::Overlay
    );
    assert_eq!(host.elements[&row_children[1]].style.grow, 2.0);
    let layers = &host.children[&Some(overlay)];
    assert_eq!(host.elements[&layers[0]].kind, ElementKind::Image);
    assert_eq!(host.elements[&layers[1]].kind, ElementKind::Text);
}

#[test]
fn changing_scroll_axis_retains_the_viewport_content_and_observer() {
    let calls = Rc::new(RefCell::new(0));
    let make_view = |axis| {
        let calls = calls.clone();
        let viewport: ScrollView = text("content")
            .scrollable()
            .height(120.0)
            .follow(ScrollFollow::EndWhenPinned)
            .on_scroll(move |_, _| *calls.borrow_mut() += 1);
        match axis {
            Some(axis) => viewport.axis(axis),
            None => viewport,
        }
    };
    let mut app = App::new(make_view(None));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    let child = host.children[&Some(root)][0];
    assert_eq!(
        host.elements[&root].style.layout,
        refract_core::Layout::Scroll(Axis::Vertical)
    );
    for axis in [Axis::Horizontal, Axis::Both, Axis::Vertical] {
        app.set_view(make_view(Some(axis)));
        host.flush(&mut app).unwrap();
        assert_eq!(host.children[&None], [root]);
        assert_eq!(host.children[&Some(root)], [child]);
        let element = &host.elements[&root];
        assert_eq!(element.style.layout, refract_core::Layout::Scroll(axis));
        assert_eq!(element.style.height, Some(120.0));
        assert_eq!(element.scroll.unwrap().follow, ScrollFollow::EndWhenPinned);
        assert!(app.dispatch(
            root,
            Event::Scroll(ScrollEvent {
                metrics: ScrollMetrics::default(),
                delta: ScrollOffset::default(),
            })
        ));
    }
    assert_eq!(*calls.borrow(), 3);
}

#[test]
fn changing_stack_direction_preserves_children_and_their_state() {
    let make_view = |direction| {
        let content: Stack = stack()
            .width(300.0)
            .gap(12.0)
            .alignment(Alignment::Start)
            .child(text("Name"))
            .child(view_fn(|cx| {
                let name = cx.state("name", || String::from("initial"));
                Ok(field(name).label("Name"))
            }));
        match direction {
            Some(direction) => content.direction(direction),
            None => content,
        }
    };
    let mut app = App::new(make_view(None));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    let children = host.children[&Some(root)].clone();
    let input = children[1];
    assert_eq!(
        host.elements[&root].style.layout,
        refract_core::Layout::Column
    );
    assert!(app.dispatch(input, Event::Input("edited".into())));
    host.flush(&mut app).unwrap();

    for direction in [Direction::Horizontal, Direction::Vertical] {
        app.set_view(make_view(Some(direction)));
        host.flush(&mut app).unwrap();
        assert_eq!(host.children[&None], [root]);
        assert_eq!(host.children[&Some(root)], children);
        assert_eq!(host.elements[&input].value, "edited");
        let element = &host.elements[&root];
        let expected = match direction {
            Direction::Horizontal => refract_core::Layout::Row,
            Direction::Vertical => refract_core::Layout::Column,
        };
        assert_eq!(element.style.layout, expected);
        assert_eq!(element.style.width, Some(300.0));
        assert_eq!(element.style.gap, 12.0);
        assert_eq!(element.style.align, Alignment::Start);
        assert!(matches!(host.updates.last().unwrap().patches.as_slice(),
            [refract_core::Patch::Update(id, _)] if *id == root));
    }
}

#[test]
fn border_and_corner_values_preserve_builders_and_update_without_wrappers() {
    let border = Border::all(1.0, Color::BLACK).bottom(3.0, Color::BLUE);
    let corners = CornerRadii::all(12.0).top_left(0.0);
    let build = |border, corners| {
        let container: Container = view().border(border).radius(corners);
        container.child(text("Content"))
    };
    let mut app = App::new(build(border, corners));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = host.children[&None][0];
    assert_eq!(host.elements.len(), 2);
    assert_eq!(host.elements[&id].style.border, Some(border));
    assert_eq!(host.elements[&id].style.radius, corners);
    app.set_view(build(Border::default().left(2.0, Color::BLUE), 6.0.into()));
    host.flush(&mut app).unwrap();
    assert_eq!(host.children[&None], [id]);
    assert_eq!(host.elements.len(), 2);
    assert_eq!(host.elements[&id].style.radius, CornerRadii::all(6.0));
    assert_eq!(host.elements[&id].style.border.unwrap().bottom.width, 0.0);
}

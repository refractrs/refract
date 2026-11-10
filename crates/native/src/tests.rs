//! Backend conformance without a window or GPU. Rendering consumes the same Frame
//! in production; these tests exercise real Taffy layout and Parley shaping.

use crate::{
    host::Host,
    renderer::{DrawCommand, Point},
    scheduler::{ScheduleError, Scheduler},
    *,
};
use refract::prelude::*;
use refract_core::{App, ElementId, Patch, Update};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

fn flush(app: &mut App, host: &mut Host) {
    if let Some(pending) = app.prepare().unwrap() {
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
    }
}

fn frame(host: &mut Host) -> renderer::Frame {
    host.frame(600.0, 700.0, 2.0, Color::WHITE).unwrap()
}

#[test]
fn input_mode_changes_reset_focused_composition_and_reconfigure_the_editor() {
    use refract_core::{ElementKind, Event};
    use winit::event::Ime;

    fn form(multiline: bool) -> impl View {
        view_fn(move |cx| {
            let value = cx.state("value", || "draft".to_owned());
            Ok(field(value).multiline(multiline).key("editor"))
        })
    }
    let input_id = |app: &App| {
        app.elements()
            .into_iter()
            .find(|(_, e)| e.kind == ElementKind::TextInput)
            .unwrap()
            .0
    };
    let mut app = App::new(form(false));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let mut id = input_id(&app);

    for multiline in [true, false] {
        host.set_focus(Some(id));
        assert!(host
            .editor_ime(&Ime::Preedit("未".into(), Some((0, 3))))
            .is_none());
        app.set_view(form(multiline));
        flush(&mut app, &mut host);
        frame(&mut host);
        let next = input_id(&app);
        assert_ne!(next, id);
        assert!(!host.is_editor(id));
        assert!(host.is_editor(next));
        assert_eq!(host.focus, None);
        assert!(host.editor_ime(&Ime::Commit("stale".into())).is_none());
        assert_eq!(
            app.elements()
                .iter()
                .find(|(i, _)| *i == next)
                .unwrap()
                .1
                .value,
            "draft"
        );

        host.set_focus(Some(next));
        let Some(Event::Input(value)) = host.editor_ime(&Ime::Commit("a\nb".into())) else {
            panic!("replacement editor must accept input");
        };
        assert_eq!(value.contains('\n'), multiline);
        assert!(
            !value.contains('未'),
            "old preedit must not leak into the new editor"
        );
        id = next;
    }
}

#[test]
fn text_tick_retains_identity_and_shapes_only_changed_text() {
    let binding = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let binding = binding.clone();
        move |cx| {
            let clock = cx.state("clock", || "12:00:00".to_string());
            *binding.borrow_mut() = Some(clock.clone());
            Ok(v_stack()
                .child(view_fn(move |cx| Ok(text(cx.read(&clock)))))
                .children((0..100).map(|i| text(format!("unchanged {i}")))))
        }
    }));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let before = host.stats();
    let ids: Vec<_> = app.elements().iter().map(|(id, _)| *id).collect();
    app.mutate(|rt| {
        binding
            .borrow()
            .as_ref()
            .unwrap()
            .set(rt, "12:00:01".into())
    });
    let pending = app.prepare().unwrap().unwrap();
    assert!(matches!(pending.patches(), [Patch::Update(_, _)]));
    assert_eq!(pending.update.stats.evaluated, 2);
    host.apply(&pending.update).unwrap();
    app.commit(pending).unwrap();
    frame(&mut host);
    assert_eq!(host.stats().text_shapes, before.text_shapes + 1);
    assert_eq!(host.stats().elements, before.elements);
    assert_eq!(
        app.elements().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ids
    );
    // Expose/redraw uses retained layouts and does not reshape or re-layout.
    let after = host.stats();
    frame(&mut host);
    assert_eq!(host.stats().text_shapes, after.text_shapes);
    assert_eq!(host.stats().layout_passes, after.layout_passes);
}

#[test]
fn paint_changes_do_not_reshape_text_or_recompute_layout() {
    fn content(color: Color) -> impl View {
        v_stack().child(text("inherited color")).foreground(color)
    }
    let mut app = App::new(content(Color::BLACK));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let before = host.stats();
    app.set_view(content(Color::BLUE));
    flush(&mut app, &mut host);
    let frame = frame(&mut host);
    assert_eq!(host.stats().text_shapes, before.text_shapes);
    assert_eq!(host.stats().layout_passes, before.layout_passes);
    assert!(frame
        .commands
        .iter()
        .any(|c| matches!(c, DrawCommand::Glyphs { color, .. } if *color == Color::BLUE)));
}

#[test]
fn inherited_typography_changes_reshape_text_and_reset_without_stale_metrics() {
    let content = |style: refract_core::Style| {
        v_stack()
            .child(text("ONE TWO").width(400.0))
            .modify(false, move |element| {
                element.style.italic = style.italic;
                element.style.letter_spacing = style.letter_spacing;
                element.style.word_spacing = style.word_spacing;
            })
    };
    let mut app = App::new(content(Default::default()));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let last_x = |frame: renderer::Frame| {
        frame
            .commands
            .iter()
            .filter_map(|command| {
                if let DrawCommand::Glyphs { run, .. } = command {
                    run.glyphs.last().map(|g| g.x)
                } else {
                    None
                }
            })
            .next_back()
            .unwrap()
    };
    let original_x = last_x(frame(&mut host));
    for (style, wider) in [
        (
            refract_core::Style {
                letter_spacing: Some(2.0),
                ..Default::default()
            },
            Some(true),
        ),
        (
            refract_core::Style {
                word_spacing: Some(5.0),
                ..Default::default()
            },
            Some(true),
        ),
        (
            refract_core::Style {
                letter_spacing: Some(-0.5),
                word_spacing: Some(-1.0),
                ..Default::default()
            },
            Some(false),
        ),
        (
            refract_core::Style {
                italic: Some(true),
                ..Default::default()
            },
            None,
        ),
        (
            refract_core::Style {
                italic: Some(false),
                letter_spacing: Some(0.0),
                word_spacing: Some(0.0),
                ..Default::default()
            },
            None,
        ),
    ] {
        let before = host.stats();
        app.set_view(content(style));
        flush(&mut app, &mut host);
        let x = last_x(frame(&mut host));
        assert_eq!(host.stats().text_shapes, before.text_shapes + 1);
        assert!(host.stats().layout_passes > before.layout_passes);
        if let Some(wider) = wider {
            assert_eq!(x > original_x, wider);
        }
        let after = host.stats();
        frame(&mut host);
        assert_eq!(host.stats().text_shapes, after.text_shapes);
        assert_eq!(host.stats().layout_passes, after.layout_passes);
    }
    assert!((last_x(frame(&mut host)) - original_x).abs() < 0.001);
}

#[test]
fn shadows_respect_opacity_and_ancestor_clips_without_affecting_layout() {
    fn content(shadow: Option<BoxShadow>) -> impl View {
        text("shadow").modify(false, move |element| {
            element.style = refract::Style {
                width: Some(200.0),
                height: Some(60.0),
                radius: 14.0.into(),
                background: Some(Color::WHITE),
                shadow,
                clipped: true,
                opacity: Some(0.5),
                ..Default::default()
            }
        })
    }
    let shadow = BoxShadow::new(0.0, 4.0, 20.0, Color(43, 48, 42, 22));
    let mut app = App::new(content(Some(shadow)));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let rendered = frame(&mut host);
    assert!(
        matches!(rendered.commands[0], DrawCommand::PushLayer { clip, opacity: 0.5, .. }
        if clip.width == 600.0 && clip.height == 700.0)
    );
    assert!(
        matches!(rendered.commands[1], DrawCommand::BoxShadow { shadow: actual, radius, .. } if actual == shadow && radius == 14.0.into())
    );
    assert!(
        matches!(rendered.commands[2], DrawCommand::PushLayer { clip, opacity: 1.0, .. }
        if clip.width == 200.0 && clip.height == 60.0)
    );
    assert!(matches!(rendered.commands[3], DrawCommand::Fill { .. }));
    assert_eq!(
        rendered
            .commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::PushLayer { .. }))
            .count(),
        rendered
            .commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::PopLayer))
            .count()
    );
    let before = host.stats();
    for next in [Some(BoxShadow::new(-2.0, 1.0, 0.0, Color::BLACK)), None] {
        app.set_view(content(next));
        let pending = app.prepare().unwrap().unwrap();
        assert!(matches!(pending.patches(), [Patch::Update(_, _)]));
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        let rendered = frame(&mut host);
        assert_eq!(
            rendered.commands.iter().find_map(|c| match c {
                DrawCommand::BoxShadow { shadow, .. } => Some(*shadow),
                _ => None,
            }),
            next
        );
        assert_eq!(host.stats().layout_passes, before.layout_passes);
        assert_eq!(host.stats().text_shapes, before.text_shapes);
    }
}

#[test]
fn blur_updates_preserve_geometry_input_and_text_caches() {
    fn content(blur: Option<f32>, backdrop_blur: Option<f32>) -> impl View {
        text("filtered")
            .width(200.0)
            .height(60.0)
            .radius(12.0)
            .opacity(0.5)
            .clipped()
            .on_press(|_| {})
            .modify(false, move |element| {
                element.style.blur = blur;
                element.style.backdrop_blur = backdrop_blur;
            })
    }
    let mut app = App::new(content(Some(4.0), Some(12.0)));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let rendered = frame(&mut host);
    assert!(matches!(rendered.commands[0], DrawCommand::PushEffects {
        blur: 4.0, backdrop_blur: 12.0, opacity: 0.5, radius, ..
    } if radius == 12.0.into()));
    assert!(matches!(
        rendered.commands[1],
        DrawCommand::PushLayer { opacity: 1.0, .. }
    ));
    let hit = host.hit_test(Point { x: 300.0, y: 350.0 });
    assert!(hit.is_some());
    assert!(host.hit_test(Point { x: 405.0, y: 350.0 }).is_none());
    let before = host.stats();
    for (blur, backdrop) in [
        (Some(8.0), None),
        (None, Some(5.0)),
        (Some(0.0), Some(0.0)),
        (None, None),
    ] {
        app.set_view(content(blur, backdrop));
        flush(&mut app, &mut host);
        let rendered = frame(&mut host);
        assert_eq!(host.hit_test(Point { x: 300.0, y: 350.0 }), hit);
        assert_eq!(host.stats().layout_passes, before.layout_passes);
        assert_eq!(host.stats().text_shapes, before.text_shapes);
        assert_eq!(
            rendered
                .commands
                .iter()
                .any(|c| matches!(c, DrawCommand::PushEffects { .. })),
            blur.is_some_and(|v| v > 0.0) || backdrop.is_some_and(|v| v > 0.0)
        );
        let pushes = rendered
            .commands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    DrawCommand::PushEffects { .. }
                        | DrawCommand::PushLayer { .. }
                        | DrawCommand::PushInsetLayer { .. }
                )
            })
            .count();
        assert_eq!(
            pushes,
            rendered
                .commands
                .iter()
                .filter(|c| matches!(c, DrawCommand::PopLayer))
                .count()
        );
    }
}

#[test]
fn gradient_updates_and_removal_are_paint_only() {
    fn content(gradient: Option<Gradient>) -> impl View {
        text("gradient").modify(false, move |element| {
            element.style = refract::Style {
                width: Some(200.0),
                height: Some(60.0),
                radius: 9.0.into(),
                background: Some(Color::BLACK),
                background_gradient: gradient.clone(),
                ..Default::default()
            }
        })
    }
    let first = LinearGradient::new(160.0, Color::BLUE, Color::WHITE);
    let mut app = App::new(content(Some(first.clone().into())));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let rendered = frame(&mut host);
    assert!(rendered.commands.iter().any(|command| matches!(command,
        DrawCommand::LinearGradient { rect, radius, gradient }
        if rect.width == 200.0 && rect.height == 60.0 && *radius == 9.0.into() && gradient == &first)));
    let before = host.stats();
    let second = LinearGradient {
        angle: 90.0,
        stops: vec![
            GradientStop::new(0.0, Color(0, 0, 0, 0)),
            GradientStop::new(0.4, Color::BLUE),
            GradientStop::new(1.0, Color::WHITE),
        ],
    };
    app.set_view(content(Some(second.clone().into())));
    let pending = app.prepare().unwrap().unwrap();
    assert!(matches!(pending.patches(), [Patch::Update(_, _)]));
    host.apply(&pending.update).unwrap();
    app.commit(pending).unwrap();
    assert!(frame(&mut host)
        .commands
        .iter()
        .any(|command| matches!(command,
        DrawCommand::LinearGradient { gradient, .. } if gradient == &second)));
    for radial in [
        RadialGradient::new(40.0, Color::BLUE, Color(0, 0, 0, 0)).center(0.25, 0.75),
        RadialGradient {
            center: (-0.25, 1.5),
            radius: 100.0,
            stops: vec![
                GradientStop::new(0.0, Color(0, 0, 0, 0)),
                GradientStop::new(0.5, Color::BLUE),
                GradientStop::new(0.5, Color::WHITE),
            ],
        },
    ] {
        app.set_view(content(Some(radial.clone().into())));
        let pending = app.prepare().unwrap().unwrap();
        assert!(matches!(pending.patches(), [Patch::Update(_, _)]));
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        let rendered = frame(&mut host);
        let index = rendered.commands.iter().position(|command| matches!(command,
            DrawCommand::RadialGradient { rect, radius, gradient }
            if rect.width == 200.0 && rect.height == 60.0 && *radius == 9.0.into() && gradient == &radial
        )).unwrap();
        assert!(matches!(
            rendered.commands[index - 1],
            DrawCommand::Fill {
                color: Color::BLACK,
                ..
            }
        ));
        assert!(!rendered
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::LinearGradient { .. })));
    }
    app.set_view(content(None));
    flush(&mut app, &mut host);
    assert!(!frame(&mut host).commands.iter().any(|command| matches!(
        command,
        DrawCommand::LinearGradient { .. } | DrawCommand::RadialGradient { .. }
    )));
    assert_eq!(host.stats().layout_passes, before.layout_passes);
    assert_eq!(host.stats().text_shapes, before.text_shapes);
}

#[test]
fn keyed_reorder_and_removal_keep_valid_host_topology() {
    fn rows(order: &[i32]) -> impl View {
        v_stack().children(
            order
                .iter()
                .map(|id| text(id.to_string()).key(*id))
                .collect::<Vec<_>>(),
        )
    }
    let mut app = App::new(rows(&[1, 2, 3]));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let before = host.stats().text_shapes;
    app.set_view(rows(&[3, 1]));
    flush(&mut app, &mut host);
    frame(&mut host);
    assert_eq!(host.stats().text_shapes, before);
    assert_eq!(host.stats().elements, 3);
}

#[test]
fn invalid_batch_is_rejected_before_mutation() {
    let mut app = App::new(text("accepted"));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let before = host.stats();
    let element = app.elements()[0].1.clone();
    let malformed = Update {
        base_revision: before.revision,
        revision: before.revision + 1,
        patches: vec![
            Patch::Create(ElementId(999), element),
            Patch::Children(None, vec![ElementId(1000)]),
        ],
        stats: Default::default(),
    };
    assert!(host.apply(&malformed).is_err());
    assert_eq!(host.stats().revision, before.revision);
    assert_eq!(host.stats().elements, before.elements);
}

#[test]
fn button_hit_testing_and_keyboard_focus_use_retained_geometry() {
    let presses = Rc::new(Cell::new(0));
    let mut app = App::new(
        v_stack().child(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(text("press"))
                .on_press({
                    let presses = presses.clone();
                    move |_| presses.set(presses.get() + 1)
                })
                .width(100.0)
                .height(40.0),
        ),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let id = host.hit_test(Point { x: 300.0, y: 350.0 }).unwrap();
    host.focus_next(false);
    assert_eq!(host.focus, Some(id));
    app.dispatch(id, host.activation(id).unwrap());
    assert_eq!(presses.get(), 1);
    app.set_view(Empty);
    flush(&mut app, &mut host);
    assert!(host.activation(id).is_none());
    assert!(host.hit_test(Point { x: 300.0, y: 350.0 }).is_none());
    assert!(!host.scroll(Point { x: 300.0, y: 350.0 }, Point { x: 0.0, y: -10.0 }));
    host.focus_next(false);
    assert_eq!(host.focus, None);
}

#[test]
fn visual_primitives_share_activation_hit_testing_focus_and_disabled_state() {
    use refract_core::ElementKind;
    for kind in [ElementKind::Box, ElementKind::Text, ElementKind::Image] {
        for disabled in [false, true] {
            let presses = Rc::new(Cell::new(0));
            let callback = presses.clone();
            macro_rules! activate {
                ($primitive:expr) => {
                    $primitive
                        .width(100.0)
                        .height(40.0)
                        .on_press(move |_| callback.set(callback.get() + 1))
                        .disabled(disabled)
                        .into_any()
                };
            }
            let primitive = match kind {
                ElementKind::Box => activate!(view()),
                ElementKind::Text => activate!(text("Press")),
                ElementKind::Image => {
                    activate!(image(crate::image::tests::png_source()).label("Press"))
                }
                _ => unreachable!(),
            };
            let mut app = App::new(primitive);
            let mut host = Host::new().unwrap();
            flush(&mut app, &mut host);
            frame(&mut host);
            let id = app.elements()[0].0;
            let point = Point { x: 300.0, y: 350.0 };
            assert_eq!(host.hit_test(point), (!disabled).then_some(id));
            host.focus_next(false);
            assert_eq!(host.focus, (!disabled).then_some(id));
            if let Some(event) = host.activation(id) {
                app.dispatch(id, event);
            }
            assert_eq!(presses.get(), if disabled { 0 } else { 1 });
        }
    }
}

fn live_scope() -> (App, refract_core::ScopeToken) {
    let captured = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let captured = captured.clone();
        move |cx| {
            *captured.borrow_mut() = Some(cx.scope());
            Ok(Empty)
        }
    }));
    let pending = app.prepare().unwrap().unwrap();
    app.commit(pending).unwrap();
    let scope = captured.borrow().clone().unwrap();
    (app, scope)
}

#[test]
fn timers_coalesce_missed_periods_and_cancel_on_drop_or_unmount() {
    let scheduler = Scheduler::new(|| {});
    let (mut app, scope) = live_scope();
    let ticks = Rc::new(Cell::new(0));
    let timer = scheduler
        .handle()
        .interval(Duration::from_secs(1), scope.clone(), {
            let ticks = ticks.clone();
            move |_| ticks.set(ticks.get() + 1)
        })
        .unwrap();
    let future = Instant::now() + Duration::from_secs(20);
    scheduler.drain(app.runtime_mut(), future);
    assert_eq!(ticks.get(), 1);
    scheduler.drain(app.runtime_mut(), future);
    assert_eq!(ticks.get(), 1);
    drop(timer);
    scheduler.drain(app.runtime_mut(), future + Duration::from_secs(20));
    assert_eq!(ticks.get(), 1);
    assert!(scheduler.deadline().is_none());
    let _timer = scheduler
        .handle()
        .interval(Duration::from_secs(1), scope, |_| panic!("dead scope ran"))
        .unwrap();
    app.dispose();
    scheduler.drain(app.runtime_mut(), future);
    assert!(scheduler.deadline().is_none());
}

#[test]
fn queued_updates_check_scope_again_at_execution_and_handles_are_weak() {
    let scheduler = Scheduler::new(|| {});
    let handle = scheduler.handle();
    let (mut app, scope) = live_scope();
    handle
        .update_scoped(scope, |_| panic!("dead completion ran"))
        .unwrap();
    app.dispose();
    scheduler.drain(app.runtime_mut(), Instant::now());
    drop(scheduler);
    assert_eq!(handle.update(|_| {}), Err(ScheduleError::Disposed));
}

#[test]
fn resizing_reflows_text_without_reshaping() {
    let mut app = App::new(
        text("A long line of text that wraps when the viewport becomes narrower.").fill_width(),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let before = host.stats();
    let narrow = host.frame(120.0, 700.0, 1.0, Color::WHITE).unwrap();
    assert_eq!(host.stats().text_shapes, before.text_shapes);
    assert_eq!(host.stats().layout_passes, before.layout_passes + 1);
    let baselines: Vec<_> = narrow
        .commands
        .iter()
        .filter_map(|command| {
            if let DrawCommand::Glyphs { run, .. } = command {
                run.glyphs.first().map(|glyph| glyph.y)
            } else {
                None
            }
        })
        .collect();
    assert!(
        baselines.len() > 1,
        "narrow text must occupy multiple lines"
    );
    assert!(baselines.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn overlays_hit_the_frontmost_enabled_control() {
    let mut app = App::new(
        z_stack()
            .child(
                h_stack()
                    .alignment(Alignment::Center)
                    .justify(Alignment::Center)
                    .child(text("back"))
                    .on_press(|_| {})
                    .width(100.0)
                    .height(40.0),
            )
            .child(
                h_stack()
                    .alignment(Alignment::Center)
                    .justify(Alignment::Center)
                    .child(text("front"))
                    .on_press(|_| {})
                    .width(100.0)
                    .height(40.0),
            ),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let buttons: Vec<_> = app
        .elements()
        .iter()
        .filter_map(|(id, element)| {
            (element.interaction == Some(refract_core::Interaction::Activate)).then_some(*id)
        })
        .collect();
    assert_eq!(
        host.hit_test(Point { x: 300.0, y: 350.0 }),
        buttons.last().copied()
    );
}

#[test]
fn scrolling_clips_input_and_preserves_text_and_layout_caches() {
    let mut app = App::new(
        v_stack()
            .children((0..5).map(|i| {
                h_stack()
                    .alignment(Alignment::Center)
                    .justify(Alignment::Center)
                    .child(text(format!("row {i}")))
                    .on_press(|_| {})
                    .height(40.0)
            }))
            .scrollable()
            .width(120.0)
            .height(80.0),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let before = host.stats();
    let point = Point { x: 300.0, y: 330.0 };
    let first = host.hit_test(point).expect("first visible row");
    assert!(host.hit_test(Point { x: 300.0, y: 405.0 }).is_none());
    assert!(host.scroll(point, Point { x: 0.0, y: -80.0 }));
    frame(&mut host);
    assert_ne!(host.hit_test(point), Some(first));
    assert!(host.hit_test(point).is_some());
    assert_eq!(host.stats().text_shapes, before.text_shapes);
    assert_eq!(host.stats().layout_passes, before.layout_passes);
}

#[test]
fn image_fit_and_resizing_reuse_decoded_resources() {
    let source = crate::image::tests::png_source();
    let content = |fit| {
        image(source.clone())
            .label("Alpine lake")
            .width(100.0)
            .height(100.0)
            .fit(fit)
    };
    let mut app = App::new(content(ImageFit::Cover));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.stats().images_decoded == 0 {
        host.poll_images().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let first = frame(&mut host);
    let (first_rect, first_image) = first
        .commands
        .iter()
        .find_map(|command| {
            if let DrawCommand::Image { rect, image } = command {
                Some((*rect, image.clone()))
            } else {
                None
            }
        })
        .unwrap();
    assert!((first_rect.width - 200.0).abs() < 0.1);
    assert_eq!(first_rect.height, 100.0);
    let before = host.stats();
    app.set_view(content(ImageFit::Contain));
    flush(&mut app, &mut host);
    let next = frame(&mut host);
    let (next_rect, next_image) = next
        .commands
        .iter()
        .find_map(|command| {
            if let DrawCommand::Image { rect, image } = command {
                Some((*rect, image.clone()))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(next_rect.width, 100.0);
    assert!((next_rect.height - 50.0).abs() < 0.1);
    assert!(std::sync::Arc::ptr_eq(&first_image, &next_image));
    assert_eq!(host.stats().images_decoded, 1);
    assert_eq!(host.stats().layout_passes, before.layout_passes);
    let resized = host.frame(800.0, 800.0, 2.0, Color::BLACK).unwrap();
    assert!(resized
        .commands
        .iter()
        .any(|c| matches!(c, DrawCommand::Image { image, .. } if image.id() == first_image.id())));
    assert_eq!(host.stats().images_decoded, 1);
    app.set_view(Empty);
    flush(&mut app, &mut host);
    assert!(!frame(&mut host)
        .commands
        .iter()
        .any(|c| matches!(c, DrawCommand::Image { .. })));
}

#[test]
fn image_intrinsic_ratio_and_failed_replacement_preserve_the_element() {
    let source = crate::image::tests::png_source();
    let mut app = App::new(image(source).label("Image unavailable").width(80.0));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let id = app.elements()[0].0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.stats().images_decoded == 0 {
        host.poll_images().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let rendered = frame(&mut host);
    let rect = rendered
        .commands
        .iter()
        .find_map(|command| {
            if let DrawCommand::Image { rect, .. } = command {
                Some(rect)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!((rect.width, rect.height), (80.0, 40.0));
    app.set_view(
        image(ImageSource::embedded(
            ImageFormat::Png,
            b"corrupt".as_slice(),
        ))
        .label("Image unavailable")
        .width(80.0),
    );
    flush(&mut app, &mut host);
    assert_eq!(app.elements()[0].0, id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.stats().text_shapes == 0 {
        host.poll_images().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let failed = frame(&mut host);
    assert!(!failed
        .commands
        .iter()
        .any(|command| matches!(command, DrawCommand::Image { .. })));
    assert!(failed
        .commands
        .iter()
        .any(|command| matches!(command, DrawCommand::Glyphs { .. })));
    assert_eq!(host.stats().images_decoded, 1);
    // The old source's pixels never replace the current error result.
    assert!(!host.poll_images().unwrap());
}

#[test]
fn scroll_follow_tracks_growth_but_respects_reading_older_content() {
    let binding = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let binding = binding.clone();
        move |cx| {
            let content = cx.state("content", || "line\n".repeat(60));
            *binding.borrow_mut() = Some(content.clone());
            Ok(view_fn(move |cx| Ok(text(cx.read(&content))))
                .scrollable()
                .initial_position(ScrollPosition::End)
                .follow(ScrollFollow::EndWhenPinned)
                .width(500.0)
                .height(300.0))
        }
    }));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    // At the bottom there is no more downward scroll, but upward scrolling works.
    let point = Point { x: 300.0, y: 350.0 };
    assert!(!host.scroll(point, Point { x: 0.0, y: -80.0 }));
    assert!(host.scroll(point, Point { x: 0.0, y: 160.0 }));
    app.mutate(|rt| {
        binding
            .borrow()
            .as_ref()
            .unwrap()
            .set(rt, "line\n".repeat(70))
    });
    flush(&mut app, &mut host);
    frame(&mut host);
    assert!(
        host.scroll(point, Point { x: 0.0, y: -40.0 }),
        "growth must not snap an upward-scrolled reader to the bottom"
    );
    app.mutate(|rt| binding.borrow().as_ref().unwrap().set(rt, "short".into()));
    flush(&mut app, &mut host);
    frame(&mut host);
    assert!(
        !host.scroll(point, Point { x: 0.0, y: 100.0 }),
        "shrinking clamps stale offsets"
    );
}

#[test]
fn hover_and_press_only_change_host_paint_and_ignore_disabled_or_removed_buttons() {
    let mut app = App::new(
        h_stack()
            .child(
                h_stack()
                    .alignment(Alignment::Center)
                    .justify(Alignment::Center)
                    .child(text("Action"))
                    .on_press(|_| {})
                    .label("Action")
                    .width(100.0)
                    .height(40.0)
                    .background(Color::WHITE),
            )
            .child(
                h_stack()
                    .alignment(Alignment::Center)
                    .justify(Alignment::Center)
                    .child(text("Disabled"))
                    .on_press(|_| {})
                    .label("Disabled")
                    .disabled(true),
            ),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let find = |label| {
        app.elements()
            .iter()
            .find(|(_, e)| e.label.as_deref() == Some(label))
            .unwrap()
            .0
    };
    let action = find("Action");
    let disabled = find("Disabled");
    let stats = host.stats();
    let feedback = |frame: renderer::Frame, alpha| {
        frame
            .commands
            .iter()
            .any(|c| matches!(c, DrawCommand::Fill { color: Color(0, 0, 0, a), .. } if *a == alpha))
    };
    assert!(host.set_hover(Some(action)));
    assert!(
        !host.set_hover(Some(action)),
        "moving within a button must not request another frame"
    );
    assert!(feedback(frame(&mut host), 13));
    assert!(host.set_pressed(Some(action)));
    assert!(feedback(frame(&mut host), 26));
    assert!(host.set_pressed(None));
    assert!(feedback(frame(&mut host), 13));
    assert!(host.set_hover(Some(disabled)));
    assert!(!feedback(frame(&mut host), 13));
    assert_eq!(host.stats().text_shapes, stats.text_shapes);
    assert_eq!(host.stats().layout_passes, stats.layout_passes);
    assert_eq!(host.stats().revision, stats.revision);
    host.set_hover(Some(action));
    app.set_view(Empty);
    flush(&mut app, &mut host);
    assert!(
        !host.set_hover(Some(action)),
        "removed targets are already cleared"
    );
    assert!(!feedback(frame(&mut host), 13));
}

#[test]
fn centered_wrapping_text_resizes_without_horizontal_jumps() {
    // Chat's structure: a fixed sidebar and a centered, padded message column
    // with a percentage width and max-width. The padding wrapper sizes to its
    // child, so returning the longest wrapped line as text width causes jumps.
    let messages = v_stack()
        .child(
            text("What makes a workspace feel calm and focused?")
                .font_size(15.0)
                .foreground(Color::BLUE),
        )
        .child(
            text("A calm workspace gives your attention somewhere to land. A few deliberate choices can make a real difference.")
                .font_size(15.0),
        )
        .max_width(740.0)
        .fill_width()
        .padding(Insets::symmetric(34.0, 32.0));
    let mut app = App::new(
        h_stack()
            .child(text("sidebar").width(262.0))
            .child(
                v_stack()
                    .alignment(Alignment::Center)
                    .child(messages)
                    .fill_width()
                    .grow(1.0)
                    .min_width(0.0),
            )
            .fill_width(),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let shapes = host.stats().text_shapes;
    let origin = |frame: renderer::Frame| {
        frame
            .commands
            .iter()
            .find_map(|command| match command {
                DrawCommand::Glyphs {
                    origin,
                    color: Color::BLUE,
                    ..
                } => Some(origin.x),
                _ => None,
            })
            .expect("message heading")
    };
    let mut positions = Vec::new();
    // One physical-pixel resize increment on a 2x display. This crosses multiple
    // word-wrap thresholds as well as the column's maximum width.
    for pixel_width in 1500..3000 {
        let width = pixel_width as f32 / 2.0;
        let x = origin(host.frame(width, 700.0, 2.0, Color::WHITE).unwrap());
        if let Some(previous) = positions.last().copied() {
            let step = x - previous;
            assert!(
                (-0.001..=0.251).contains(&step),
                "width {width}: text moved from {previous} to {x}"
            );
        }
        positions.push(x);
    }
    // Cached measurements must give the same answer when shrinking again.
    for (pixel_width, expected) in (1500..3000).zip(positions).rev() {
        let width = pixel_width as f32 / 2.0;
        let x = origin(host.frame(width, 700.0, 2.0, Color::WHITE).unwrap());
        assert!(
            (x - expected).abs() < 0.001,
            "resize history changed x at {width}"
        );
    }
    assert_eq!(host.stats().text_shapes, shapes);
    assert!(app.prepare().unwrap().is_none());
}

#[test]
fn scroll_events_report_actual_coalesced_offsets_without_graph_or_layout_work() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::new(
        scroll_view(text("line\n".repeat(100)).width(800.0))
            .axis(Axis::Both)
            .on_scroll({
                let received = received.clone();
                move |_, event| received.borrow_mut().push(*event)
            })
            .width(300.0)
            .height(200.0),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let events = host.take_scroll_events();
    assert_eq!(events.len(), 1);
    let id = events[0].0;
    for (id, event) in events {
        app.dispatch(id, event);
    }
    let initial = received.borrow()[0];
    assert_eq!(initial.delta, ScrollOffset::default());
    assert_eq!(
        initial.metrics.viewport_size,
        ScrollSize {
            width: 300.0,
            height: 200.0
        }
    );
    assert!(initial.metrics.max_offset().x >= 500.0);
    assert!(initial.metrics.max_offset().y > 0.0);
    let before = host.stats();
    let point = Point { x: 300.0, y: 350.0 };
    for delta in [
        Point {
            x: -11.5,
            y: -12.25,
        },
        Point { x: -8.25, y: -3.5 },
    ] {
        assert!(host.scroll(point, delta));
    }
    frame(&mut host);
    let events = host.take_scroll_events();
    assert_eq!(events.len(), 1);
    for (id, event) in events {
        app.dispatch(id, event);
    }
    let moved = received.borrow()[1];
    assert_eq!(moved.delta, ScrollOffset { x: 19.75, y: 15.75 });
    assert_eq!(moved.metrics.offset, moved.delta);
    assert!(app.prepare().unwrap().is_none());
    assert_eq!(host.stats().text_shapes, before.text_shapes);
    assert_eq!(host.stats().layout_passes, before.layout_passes);
    frame(&mut host);
    assert!(host.take_scroll_events().is_empty());
    // Clamping reports only the remaining movement, and an attempted scroll at
    // the boundary emits no duplicate event.
    assert!(host.scroll(
        point,
        Point {
            x: -10000.0,
            y: -10000.0
        }
    ));
    frame(&mut host);
    let (_, event) = host.take_scroll_events().pop().unwrap();
    let refract_core::Event::Scroll(end) = &event else {
        panic!("scroll event");
    };
    assert_eq!(end.metrics.offset, end.metrics.max_offset());
    assert_eq!(end.delta.y, end.metrics.offset.y - moved.metrics.offset.y);
    assert!(!host.scroll(point, Point { x: -1.0, y: -1.0 }));
    frame(&mut host);
    assert!(host.take_scroll_events().is_empty());
    app.set_view(text("removed"));
    flush(&mut app, &mut host);
    assert!(
        !app.dispatch(id, event),
        "removed scroll targets cannot invoke callbacks"
    );
}

#[test]
fn scroll_callback_updates_only_its_reactive_consumer() {
    let mut app = App::new(view_fn(|cx| {
        let at_end = cx.state("at-end", || false);
        let observed = at_end.clone();
        Ok(v_stack()
            .child(
                scroll_view(text("line\n".repeat(100)))
                    .on_scroll(move |cx, event| {
                        observed.set_if_changed(
                            cx.runtime(),
                            event.metrics.is_at_end(Axis::Vertical, 2.0),
                        )
                    })
                    .width(300.0)
                    .height(200.0),
            )
            .child(view_fn(move |cx| {
                Ok(text(if cx.read(&at_end) {
                    "Latest"
                } else {
                    "Earlier"
                }))
            })))
    }));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    for (id, event) in host.take_scroll_events() {
        app.dispatch(id, event);
    }
    assert!(
        app.prepare().unwrap().is_none(),
        "equal state write must not invalidate views"
    );
    assert!(host.scroll(
        Point { x: 300.0, y: 340.0 },
        Point {
            x: 0.0,
            y: -10000.0
        }
    ));
    frame(&mut host);
    for (id, event) in host.take_scroll_events() {
        app.dispatch(id, event);
    }
    let pending = app.prepare().unwrap().unwrap();
    assert_eq!(pending.update.stats.evaluated, 2);
    assert!(matches!(pending.patches(), [Patch::Update(_, element)] if element.text == "Latest"));
}

#[test]
fn scroll_metrics_exclude_borders_and_notify_resizes_and_content_changes() {
    let height = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let binding = height.clone();
        move |cx| {
            let height = cx.state("height", || 800.0);
            *binding.borrow_mut() = Some(height.clone());
            Ok(
                scroll_view(view_fn(move |cx| Ok(v_stack().height(cx.read(&height)))))
                    .on_scroll(|_, _| {})
                    .fill_width()
                    .fill_height()
                    .border(Border::all(2.0, Color::BLACK)),
            )
        }
    }));
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    frame(&mut host);
    let (_, refract_core::Event::Scroll(initial)) = host.take_scroll_events().pop().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(
        initial.metrics.viewport_size,
        ScrollSize {
            width: 596.0,
            height: 696.0
        }
    );
    assert_eq!(initial.metrics.content_size.height, 800.0);
    host.frame(640.0, 650.0, 2.0, Color::WHITE).unwrap();
    let (_, refract_core::Event::Scroll(resized)) = host.take_scroll_events().pop().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(
        resized.metrics.viewport_size,
        ScrollSize {
            width: 636.0,
            height: 646.0
        }
    );
    assert_eq!(resized.delta, ScrollOffset::default());
    app.mutate(|rt| height.borrow().as_ref().unwrap().set(rt, 1000.0));
    flush(&mut app, &mut host);
    host.frame(640.0, 650.0, 2.0, Color::WHITE).unwrap();
    let (_, refract_core::Event::Scroll(grown)) = host.take_scroll_events().pop().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(grown.metrics.content_size.height, 1000.0);
    assert_eq!(grown.delta, ScrollOffset::default());
}

#[test]
fn fixed_sidebar_geometry_stays_anchored_during_two_axis_resize() {
    // A Chat-shaped sidebar: top controls, a flexible scroll region, and footer.
    // Color markers isolate top/bottom glyph origins from the changing main pane.
    let sidebar = v_stack()
        .gap(24.0)
        .child(text("refract").font_size(21.0).foreground(Color::BLUE))
        .child(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(text("New conversation"))
                .on_press(|_| {})
                .fill_width()
                .height(44.0),
        )
        .child(
            v_stack()
                .child(text("YOUR CONVERSATIONS"))
                .children(
                    [
                        "A calmer kind of workspace",
                        "Designing with intention",
                        "A weekend away",
                    ]
                    .map(|title| {
                        h_stack()
                            .alignment(Alignment::Center)
                            .justify(Alignment::Center)
                            .child(
                                h_stack()
                                    .gap(8.0)
                                    .child(text("○"))
                                    .child(text(title).font_size(12.0).grow(1.0))
                                    .padding(10.0)
                                    .fill_width(),
                            )
                            .on_press(|_| {})
                            .fill_width()
                    }),
                )
                .scrollable()
                .grow(1.0)
                .min_height(0.0),
        )
        .child(text("Your workspace").foreground(Color::rgb(240, 10, 10)))
        .fill_height()
        .padding(Insets::symmetric(18.0, 24.0))
        .width(262.0)
        .fill_height()
        .background(Color::rgb(239, 238, 232));
    let mut app = App::new(
        h_stack()
            .child(sidebar)
            .child(text("Main conversation").grow(1.0).min_width(0.0))
            .fill_width()
            .fill_height(),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let mut first_top = None;
    let mut first_footer_distance = None;
    // Alternating expansion/shrinkage catches history-dependent cached results.
    for step in (0..800).chain((0..800).rev()) {
        let width = 800.0 + step as f32 * 0.5;
        let height = 650.0 + step as f32 * 0.25;
        let frame = host.frame(width, height, 2.0, Color::WHITE).unwrap();
        for command in &frame.commands {
            match command {
                DrawCommand::Fill { rect, color, .. } if *color == Color::rgb(239, 238, 232) => {
                    assert_eq!(
                        (rect.x, rect.y, rect.width, rect.height),
                        (0.0, 0.0, 262.0, height)
                    );
                }
                DrawCommand::Glyphs {
                    origin,
                    color: Color::BLUE,
                    ..
                } => {
                    let expected = first_top.get_or_insert((origin.x, origin.y));
                    assert_eq!((origin.x, origin.y), *expected);
                }
                DrawCommand::Glyphs { origin, color, .. } if *color == Color::rgb(240, 10, 10) => {
                    let position = (origin.x, height - origin.y);
                    let expected = first_footer_distance.get_or_insert(position);
                    assert!((position.0 - expected.0).abs() < 0.001);
                    assert!((position.1 - expected.1).abs() < 0.001);
                }
                _ => {}
            }
        }
    }
    assert!(first_top.is_some() && first_footer_distance.is_some());
    assert!(app.prepare().unwrap().is_none());
}

#[test]
fn independent_borders_affect_layout_and_preserve_corners_in_paint_and_image_clips() {
    let border = Border::default()
        .top(2.0, Color::BLUE)
        .right(4.0, Color::BLACK)
        .bottom(6.0, Color::BLUE)
        .left(8.0, Color::BLACK);
    let radii = CornerRadii::all(20.0).top_left(0.0).bottom_right(8.0);
    let mut app = App::new(
        image("missing.png")
            .width(120.0)
            .height(80.0)
            .border(border)
            .radius(radii)
            .background(Color::WHITE)
            .clipped(),
    );
    let mut host = Host::new().unwrap();
    flush(&mut app, &mut host);
    let rendered = frame(&mut host);
    assert!(rendered.commands.iter().any(|cmd| matches!(cmd,
        DrawCommand::Border { border: b, radius, .. } if *b == border && *radius == radii)));
    assert!(rendered.commands.iter().any(|cmd| matches!(cmd,
        DrawCommand::PushInsetLayer { rect, radius, insets } if rect.width == 120.0
            && rect.height == 80.0 && *radius == radii
            && *insets == Insets { top: 2.0, right: 4.0, bottom: 6.0, left: 8.0 })));
    assert!(rendered.commands.iter().any(|cmd| matches!(cmd,
        DrawCommand::Fill { rect, .. } if rect.width == 108.0 && rect.height == 72.0)));
    let pushes = rendered
        .commands
        .iter()
        .filter(|cmd| {
            matches!(
                cmd,
                DrawCommand::PushLayer { .. } | DrawCommand::PushInsetLayer { .. }
            )
        })
        .count();
    assert_eq!(
        pushes,
        rendered
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, DrawCommand::PopLayer))
            .count()
    );
}

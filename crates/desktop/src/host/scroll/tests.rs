use super::*;
use refract::prelude::*;
use refract_core::App;

fn setup(view: impl View) -> (App, Host) {
    let mut app = App::new(view);
    let mut host = Host::new().unwrap();
    let pending = app.prepare().unwrap().unwrap();
    host.apply(&pending.update).unwrap();
    app.commit(pending).unwrap();
    paint(&mut host);
    (app, host)
}

fn paint(host: &mut Host) {
    host.frame(600.0, 400.0, 2.0, Color::WHITE).unwrap();
}

fn pane() -> impl View {
    v_stack()
        .children((0..12).map(|i| text(format!("row {i}")).height(40.0)))
        .width(180.0)
        .height(100.0)
        .scrollable()
}

#[test]
fn precise_deltas_capture_a_pane_and_do_not_create_graph_or_layout_work() {
    let (mut app, mut host) = setup(h_stack().child(pane()).child(pane()));
    let bars = host.scrolling.bars.clone();
    assert_eq!(bars.len(), 2);
    let point = |bar: Scrollbar| Point {
        x: bar.track.x - 40.0,
        y: bar.track.y + 20.0,
    };
    let delta = Point { x: 0.0, y: -17.25 };
    let stats = host.stats();
    assert!(host.scroll_event(point(bars[0]), delta, TouchPhase::Started, true));
    assert!(host.scroll_event(point(bars[1]), delta, TouchPhase::Moved, true));
    assert_eq!(host.entries[&bars[0].target].scroll.y, 34.5);
    assert_eq!(host.entries[&bars[1].target].scroll.y, 0.0);
    host.scroll_event(point(bars[1]), Point::default(), TouchPhase::Ended, true);
    assert!(host.scroll_event(point(bars[1]), delta, TouchPhase::Started, true));
    assert_eq!(host.entries[&bars[1].target].scroll.y, 17.25);
    // Devices without phase-end events must not retain capture forever.
    host.scrolling.last_event = Some(Instant::now() - Duration::from_secs(1));
    assert!(host.scroll_event(point(bars[0]), delta, TouchPhase::Moved, true));
    assert_eq!(host.entries[&bars[0].target].scroll.y, 51.75);
    paint(&mut host);
    assert_eq!(host.stats().layout_passes, stats.layout_passes);
    assert_eq!(host.stats().text_shapes, stats.text_shapes);
    assert!(app.prepare().unwrap().is_none());
}

#[test]
fn scrollbar_capture_clamps_outside_window_and_survives_resize() {
    let (_, mut host) = setup(pane());
    let bar = host.scrolling.bars[0];
    assert!(host.scrollbar_press(Point {
        x: bar.thumb.x + 4.0,
        y: bar.thumb.y + 4.0
    }));
    host.frame(400.0, 300.0, 1.0, Color::WHITE).unwrap();
    assert!(host.scrollbar_drag(Point {
        x: -500.0,
        y: 10000.0
    }));
    assert_eq!(host.entries[&bar.target].scroll.y, bar.max);
    assert!(host.scrollbar_release());
    assert!(!host.scrollbar_drag(Point::default()));
    assert!(!host.scroll_event(
        Point::default(),
        Point {
            x: f32::NAN,
            y: 1.0
        },
        TouchPhase::Moved,
        true
    ));
}

#[test]
fn discrete_wheels_chain_at_boundaries_and_covered_bars_do_not_capture() {
    let (_, mut host) = setup(
        v_stack()
            .child(pane())
            .child(text("tail").height(300.0))
            .width(200.0)
            .height(120.0)
            .scrollable(),
    );
    let inner = host.scrolling.bars[0];
    let outer = host.scrolling.bars[1];
    let point = Point {
        x: inner.track.x - 40.0,
        y: inner.track.y + 10.0,
    };
    host.entries.get_mut(&inner.target).unwrap().scroll.y = inner.max;
    assert!(host.scroll(point, Point { x: 0.0, y: -20.0 }));
    assert_eq!(host.entries[&outer.target].scroll.y, 20.0);
    let (_, mut host) = setup(
        z_stack()
            .child(pane())
            .child(text("cover").width(180.0).height(100.0)),
    );
    let bar = host.scrolling.bars[0];
    assert!(!host.scrollbar_press(Point {
        x: bar.thumb.x + 4.0,
        y: bar.thumb.y + 4.0
    }));
}

#[test]
fn no_overflow_means_no_scrollbar_and_removed_capture_is_safe() {
    let (_, host) = setup(
        text("fits")
            .width(180.0)
            .height(100.0)
            .scrollable().axis(Axis::Both),
    );
    assert!(host.scrolling.bars.is_empty());
    let (_, mut host) = setup(pane());
    let bar = host.scrolling.bars[0];
    let point = Point {
        x: bar.thumb.x + 4.0,
        y: bar.thumb.y + 4.0,
    };
    assert!(host.scrollbar_press(point));
    host.entries.remove(&bar.target);
    assert!(!host.scrollbar_drag(point));
    assert!(!host.scrollbar_hit(point));
}

#[test]
fn auto_hide_is_paint_only_and_removed_scrollbars_drop_their_deadlines() {
    let (mut app, mut host) = setup(pane());
    let bar = host.scrolling.bars[0];
    let point = Point {
        x: bar.track.x - 20.0,
        y: bar.track.y + 10.0,
    };
    let stats = host.stats();
    assert!(host.scrollbar_deadline().is_none(), "hidden at rest");
    assert!(host.scroll(point, Point { x: 0.0, y: -20.0 }));
    let now = Instant::now();
    host.frame_at(600.0, 400.0, 2.0, Color::WHITE, now).unwrap();
    assert!(host.scrollbar_deadline().unwrap() > now);
    host.frame_at(
        600.0,
        400.0,
        2.0,
        Color::WHITE,
        now + Duration::from_secs(2),
    )
    .unwrap();
    assert!(
        host.scrollbar_deadline().is_none(),
        "no continuous redraw once faded"
    );
    assert_eq!(host.stats().layout_passes, stats.layout_passes);
    assert_eq!(host.stats().text_shapes, stats.text_shapes);
    assert!(app.prepare().unwrap().is_none());
    // Hidden edges remain discoverable. Hover and capture suspend auto-hide.
    let edge = Point {
        x: bar.track.x + 4.0,
        y: bar.track.y + 10.0,
    };
    assert!(host.scrollbar_pointer(Some(edge)));
    assert!(host.scrollbar_press(edge));
    host.frame_at(
        600.0,
        400.0,
        2.0,
        Color::WHITE,
        now + Duration::from_secs(3),
    )
    .unwrap();
    let presentation = &host.scrolling.presentation[&(bar.target, true)];
    assert_eq!(presentation.opacity(now + Duration::from_secs(30)), 1.0);
    host.scrollbar_release();
    host.scrollbar_pointer(None);
    paint(&mut host);
    assert!(host.scrollbar_deadline().is_some());
    app.set_view(text("removed"));
    let pending = app.prepare().unwrap().unwrap();
    host.apply(&pending.update).unwrap();
    app.commit(pending).unwrap();
    paint(&mut host);
    assert!(host.scrolling.presentation.is_empty());
    assert!(host.scrollbar_deadline().is_none());
}

#[test]
fn resize_keeps_hidden_scrollbars_hidden_after_clamping_or_following() {
    for follow in [ScrollFollow::None, ScrollFollow::EndWhenPinned] {
        let (_, mut host) = setup(
            v_stack()
                .children((0..20).map(|i| text(format!("line {i}")).height(40.0)))
                .scrollable()
                .initial_position(ScrollPosition::End)
                .follow(follow)
                .width(180.0)
                .fill_height()
                .grow(1.0)
                .min_height(0.0),
        );
        let id = host.scrolling.bars[0].target;
        let now = Instant::now();
        host.scrolling.reveal(id, now);
        let hidden = now + Duration::from_secs(2);
        host.frame_at(600.0, 400.0, 2.0, Color::WHITE, hidden)
            .unwrap();
        assert!(host.scrollbar_deadline().is_none());
        let previous_offset = host.entries[&id].scroll.y;

        // Enlarging clamps the offset; shrinking follows the end only when
        // requested. Neither adjustment is user scroll activity.
        for height in [500.0, 300.0] {
            host.frame_at(600.0, height, 2.0, Color::WHITE, hidden)
                .unwrap();
            assert_ne!(host.entries[&id].scroll.y, previous_offset);
            assert_eq!(
                host.scrolling.presentation[&(id, true)].opacity(hidden),
                0.0
            );
            assert!(host.scrollbar_deadline().is_none());
            // The new offset is recorded, so the following paint stays quiet.
            host.frame_at(600.0, height, 2.0, Color::WHITE, hidden)
                .unwrap();
            assert!(host.scrollbar_deadline().is_none());
        }

        let bar = host.scrolling.bars[0];
        let edge = Point {
            x: bar.track.x + 4.0,
            y: bar.track.y + 10.0,
        };
        assert!(!host.reconcile_scrollbar_hover(Some(edge)));
        assert_eq!(
            host.scrolling.presentation[&(id, true)].opacity(hidden),
            0.0
        );

        // Real pointer movement still discovers the hidden edge, and layout
        // reconciliation can release it when the edge moves away.
        assert!(host.scrollbar_pointer(Some(edge)));
        assert_eq!(
            host.scrolling.presentation[&(id, true)].opacity(hidden),
            1.0
        );
        assert!(host.reconcile_scrollbar_hover(None));

        // Actual scrolling remains an explicit reveal even on a resize frame.
        let point = Point {
            x: bar.track.x - 20.0,
            y: bar.track.y + 10.0,
        };
        assert!(host.scroll(point, Point { x: 0.0, y: 20.0 }));
        let activity = Instant::now();
        host.frame_at(600.0, 350.0, 2.0, Color::WHITE, activity)
            .unwrap();
        assert_eq!(
            host.scrolling.presentation[&(id, true)].opacity(activity),
            1.0
        );
    }
}

#[test]
fn resize_and_thumb_animation_rebase_capture_without_moving_content() {
    let (_, mut host) = setup(
        v_stack()
            .children((0..20).map(|i| text(format!("line {i}")).height(40.0)))
            .scrollable()
            .width(180.0)
            .fill_height()
            .grow(1.0)
            .min_height(0.0),
    );
    let id = host.scrolling.bars[0].target;
    host.entries.get_mut(&id).unwrap().scroll.y = 100.0;
    paint(&mut host);
    let bar = host.scrolling.bars[0];
    let point = Point {
        x: bar.thumb.x + 4.0,
        y: bar.thumb.y + bar.thumb.height * 0.75,
    };
    host.scrollbar_press(point);
    let offset = host.entries[&bar.target].scroll.y;
    let now = Instant::now();
    host.frame_at(600.0, 300.0, 2.0, Color::WHITE, now).unwrap();
    host.scrollbar_drag(point);
    assert!((host.entries[&bar.target].scroll.y - offset).abs() < 0.001);
    host.frame_at(
        600.0,
        300.0,
        2.0,
        Color::WHITE,
        now + Duration::from_millis(70),
    )
    .unwrap();
    host.scrollbar_drag(point);
    assert!((host.entries[&bar.target].scroll.y - offset).abs() < 0.001);
    host.frame_at(
        600.0,
        300.0,
        2.0,
        Color::WHITE,
        now + Duration::from_millis(200),
    )
    .unwrap();
    host.scrollbar_drag(point);
    assert!((host.entries[&bar.target].scroll.y - offset).abs() < 0.001);
    let final_bar = host.scrolling.bars[0];
    let expected = final_bar.track.height * 300.0 / (300.0 + final_bar.max);
    assert!((final_bar.thumb.height - expected).abs() < 0.001);
}

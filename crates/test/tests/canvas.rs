use refract_core::{prelude::*, App, ElementKind};
use refract_test::TestHost;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[test]
fn drawing_inputs_are_local_and_discarded_preparations_do_not_commit() {
    let parent = Rc::new(Cell::new(0));
    let prepared = Rc::new(Cell::new(0));
    let saved = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let (parent, prepared, saved) = (parent.clone(), prepared.clone(), saved.clone());
        move |cx| {
            parent.set(parent.get() + 1);
            let value = cx.state("value", || 0.0f64);
            *saved.borrow_mut() = Some(value.clone());
            let prepared = prepared.clone();
            Ok(canvas_with(
                move |cx| {
                    prepared.set(prepared.get() + 1);
                    cx.read(&value)
                },
                |g, _, x| {
                    g.fill(
                        &Path::rectangle(Rect::new(*x, 0.0, 10.0, 10.0)),
                        Color::BLUE,
                    )
                },
            )
            .animatable_inputs()
            .width(200.0)
            .height(100.0))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = *host
        .elements
        .iter()
        .find(|(_, e)| e.kind == ElementKind::Canvas)
        .unwrap()
        .0;
    let initial = host.elements[&id].canvas.clone().unwrap();
    with_animation(
        app.runtime_mut(),
        Animation::linear(Duration::from_secs(1)),
        |rt| {
            saved.borrow().as_ref().unwrap().set(rt, 100.0);
        },
    );
    drop(app.prepare().unwrap().unwrap());
    assert_eq!(host.elements[&id].canvas.as_ref(), Some(&initial));
    host.flush(&mut app).unwrap();
    assert_eq!(parent.get(), 1);
    let count = prepared.get();
    let displayed = host.presentation.advance(Duration::from_millis(500));
    let halfway = displayed
        .iter()
        .find(|(i, _)| *i == id)
        .unwrap()
        .1
        .canvas
        .as_ref()
        .unwrap();
    let commands = halfway
        .draw(Size::new(200.0, 100.0), Duration::ZERO)
        .unwrap();
    assert_eq!(drawing_bounds(&commands).unwrap().x, 50.0);
    assert_eq!(prepared.get(), count);
    assert!(app.prepare().unwrap().is_none());
    host.presentation.advance(Duration::from_secs(1));
    assert!(!host.presentation.is_animating());
}

#[test]
fn drawing_cache_reuses_static_work_and_pauses_scheduled_time() {
    let count = Rc::new(Cell::new(0));
    let mut app = App::new(
        canvas({
            let count = count.clone();
            move |g, frame| {
                count.set(count.get() + 1);
                g.fill(
                    &Path::rectangle(Rect::new(frame.elapsed.as_secs_f64(), 0.0, 10.0, 10.0)),
                    Color::BLACK,
                );
            }
        })
        .schedule(CanvasSchedule::animation()),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let content = host
        .elements
        .values()
        .find_map(|e| e.canvas.as_ref())
        .unwrap();
    let mut cache = CanvasCache::default();
    let size = Size::new(100.0, 100.0);
    let a = cache
        .render(content, size, Duration::ZERO, true, false)
        .unwrap();
    let b = cache
        .render(content, size, Duration::ZERO, true, false)
        .unwrap();
    assert!(Rc::ptr_eq(&a, &b));
    let c = cache
        .render(content, size, Duration::from_secs(1), true, false)
        .unwrap();
    assert_eq!(drawing_bounds(&c).unwrap().x, 1.0);
    cache.pause();
    let d = cache
        .render(content, size, Duration::from_secs(30), true, false)
        .unwrap();
    assert!(Rc::ptr_eq(&c, &d));
    let reduced = cache
        .render(content, size, Duration::from_secs(31), true, true)
        .unwrap();
    assert_eq!(drawing_bounds(&reduced).unwrap().x, 0.0);
    assert_eq!(count.get(), 3);
}

#[test]
fn drawing_subscriptions_follow_dynamic_reads_and_release_on_removal() {
    let saved = Rc::new(RefCell::new(None));
    let runs = Rc::new(Cell::new(0));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        let runs = runs.clone();
        move |cx| {
            let select = cx.state("select", || false);
            let a = cx.state("a", || 1.0f64);
            let b = cx.state("b", || 2.0f64);
            *saved.borrow_mut() = Some((select.clone(), a.clone(), b.clone()));
            let runs = runs.clone();
            Ok(canvas_with(
                move |cx| {
                    runs.set(runs.get() + 1);
                    if cx.read(&select) {
                        cx.read(&b)
                    } else {
                        cx.read(&a)
                    }
                },
                |_, _, _| {},
            ))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let (select, a, b) = saved.borrow().clone().unwrap();
    b.set(app.runtime_mut(), 3.0);
    assert!(!host.flush(&mut app).unwrap());
    select.set(app.runtime_mut(), true);
    host.flush(&mut app).unwrap();
    a.set(app.runtime_mut(), 4.0);
    assert!(!host.flush(&mut app).unwrap());
    assert_eq!(runs.get(), 2);
    app.set_view(Empty);
    host.flush(&mut app).unwrap();
    b.set(app.runtime_mut(), 8.0);
    assert!(!host.flush(&mut app).unwrap());
    assert!(!host.presentation.is_animating());
}

#[test]
fn pointer_geometry_uses_each_handler_coordinate_system() {
    let positions = Rc::new(RefCell::new(Vec::new()));
    let capture = positions.clone();
    let mut app = App::new(view().on_pointer(move |cx, event| {
        capture
            .borrow_mut()
            .push((cx.local_position(event.position), cx.local_bounds()));
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = *host.elements.keys().next().unwrap();
    let event = refract_core::Event::Pointer(PointerEvent {
        id: 1,
        kind: PointerKind::Mouse,
        phase: PointerPhase::Move,
        position: EventPoint { x: 120.0, y: 80.0 },
        button: None,
        buttons: PointerButtons(0),
        modifiers: Default::default(),
        pressure: 0.0,
    });
    app.dispatch_event_with_geometry(id, event.clone(), |_| {
        Some(refract_core::ElementGeometry {
            bounds: Rect::new(0.0, 0.0, 100.0, 50.0),
            transform: refract_core::Affine2d([2.0, 0.0, 0.0, 2.0, 100.0, 60.0]),
        })
    });
    assert_eq!(
        positions.borrow()[0],
        (
            Some(Point::new(10.0, 10.0)),
            Some(Rect::new(0.0, 0.0, 100.0, 50.0))
        )
    );
    app.dispatch_event(id, event);
    assert_eq!(positions.borrow()[1], (None, None));
}

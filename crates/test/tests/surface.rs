use refract_core::*;
use refract_test::TestHost;
use std::{cell::Cell, rc::Rc};

fn viewport(width: f64) -> Viewport {
    Viewport {
        width,
        height: Some(600.0),
        scale_factor: 1.0,
    }
}

#[test]
fn observations_invalidate_only_readers_and_equal_samples_do_nothing() {
    let titles = Rc::new(Cell::new(0));
    let sizes = Rc::new(Cell::new(0));
    let title_view = view_fn({
        let calls = titles.clone();
        move |cx| {
            let surface = cx.object::<Surface>()?;
            calls.set(calls.get() + 1);
            Ok(text(cx.read(&surface.title)))
        }
    });
    let size_view = view_fn({
        let calls = sizes.clone();
        move |cx| {
            let surface = cx.object::<Surface>()?;
            calls.set(calls.get() + 1);
            Ok(text(format!("{:?}", cx.read(&surface.viewport))))
        }
    });
    let mut app = App::new(v_stack().child(title_view).child(size_view));
    let mut host = TestHost::default();
    host.install_surface(&mut app, "Initial", None);
    host.set_viewport(&mut app, viewport(900.0)).unwrap();
    host.flush(&mut app).unwrap();
    assert_eq!((titles.get(), sizes.get()), (1, 1));
    host.set_viewport(&mut app, viewport(500.0)).unwrap();
    host.flush(&mut app).unwrap();
    assert_eq!((titles.get(), sizes.get()), (1, 2));
    host.set_viewport(&mut app, viewport(500.0)).unwrap();
    assert!(!host.flush(&mut app).unwrap());
    assert_eq!(sizes.get(), 2);
    host.surface
        .as_ref()
        .unwrap()
        .publish_title(app.runtime_mut(), "External".into());
    host.flush(&mut app).unwrap();
    assert_eq!((titles.get(), sizes.get()), (2, 2));
    assert!(host
        .surface
        .as_ref()
        .unwrap()
        .take_requests(app.runtime_mut())
        .is_empty());
}

#[test]
fn host_only_requests_work_without_view_patches_and_keep_navigation_order() {
    let mut app = App::new(text("unchanged"));
    let mut host = TestHost::default();
    let surface = host.install_surface(
        &mut app,
        "Initial",
        Some("https://example.com/base/".parse().unwrap()),
    );
    host.flush(&mut app).unwrap();
    let title = surface.set_title(app.runtime_mut(), "Updated").unwrap();
    let first = surface
        .navigate(app.runtime_mut(), "one/", HistoryMode::Push)
        .unwrap();
    surface
        .navigate(app.runtime_mut(), "two", HistoryMode::Push)
        .unwrap();
    assert!(!host.flush(&mut app).unwrap());
    assert_eq!(title.status.get(app.runtime()), RequestStatus::Applied);
    assert_eq!(first.status.get(app.runtime()), RequestStatus::Applied);
    assert_eq!(surface.title.get(app.runtime()), "Updated");
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().path(),
        "/base/one/two"
    );
    host.go(&mut app, -1);
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().path(),
        "/base/one/"
    );
    surface
        .navigate(app.runtime_mut(), "?replaced", HistoryMode::Replace)
        .unwrap();
    host.flush(&mut app).unwrap();
    host.go(&mut app, -1);
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().path(),
        "/base/"
    );
    host.go(&mut app, 1);
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().query(),
        Some("replaced")
    );
}

#[test]
fn failed_navigation_preserves_location_and_close_rejects_pending_work() {
    let mut app = App::new(Empty);
    let mut host = TestHost::default();
    let surface = host.install_surface(&mut app, "Initial", None);
    host.flush(&mut app).unwrap();
    let request = surface
        .navigate(app.runtime_mut(), "/relative", HistoryMode::Push)
        .unwrap();
    host.flush(&mut app).unwrap();
    assert!(matches!(
        request.status.get(app.runtime()),
        RequestStatus::Failed(SurfaceError::InvalidUrl(_))
    ));
    assert_eq!(surface.location.get(app.runtime()), None);
    let pending = surface.set_title(app.runtime_mut(), "Unapplied").unwrap();
    host.surface.as_ref().unwrap().close(app.runtime_mut());
    assert_eq!(
        pending.status.get(app.runtime()),
        RequestStatus::Failed(SurfaceError::Closed)
    );
    assert!(matches!(
        surface.set_title(app.runtime_mut(), "Late"),
        Err(SurfaceError::Closed)
    ));
    assert_eq!(surface.title.get(app.runtime()), "Initial");
}

#[test]
fn discarded_preparation_does_not_issue_title_or_history_changes() {
    let mut app = App::new(view_fn(|cx| {
        let surface = cx.object::<Surface>()?;
        cx.effect("metadata", (), move |rt| {
            surface.set_title(rt, "Committed").unwrap();
            surface.navigate(rt, "/next", HistoryMode::Push).unwrap();
            Box::new(|| {})
        });
        Ok(Empty)
    }));
    let driver = SurfaceDriver::install(
        &mut app,
        "Initial".into(),
        Some("https://example.com/".parse().unwrap()),
        SurfaceCapabilities {
            title: true,
            navigation: true,
        },
    );
    drop(app.prepare().unwrap());
    assert!(driver.take_requests(app.runtime_mut()).is_empty());
    let pending = app.prepare().unwrap().unwrap();
    app.commit(pending).unwrap();
    assert_eq!(driver.take_requests(app.runtime_mut()).len(), 2);
}

#[test]
fn read_only_handles_and_breakpoints_use_existing_graph_dependencies() {
    let mut app = App::new(Empty);
    let driver = SurfaceDriver::install(
        &mut app,
        String::new(),
        None,
        SurfaceCapabilities::default(),
    );
    let surface = driver.surface();
    let compact = Derived::distinct(app.runtime_mut(), move |rt| {
        surface
            .viewport
            .get(rt)
            .is_some_and(|viewport| viewport.width < 760.0)
    });
    let observer = app.runtime_mut().observer();
    let (_, dependencies) = app.runtime().capture(|rt| *compact.get(rt));
    app.runtime_mut().accept(&observer, dependencies);
    driver
        .publish_viewport(app.runtime_mut(), viewport(1000.0))
        .unwrap();
    assert!(!observer.is_dirty(app.runtime()));
    driver
        .publish_viewport(app.runtime_mut(), viewport(600.0))
        .unwrap();
    assert!(observer.is_dirty(app.runtime()));
    app.runtime_mut().mark_clean(&observer);
    driver
        .publish_viewport(app.runtime_mut(), viewport(500.0))
        .unwrap();
    assert!(!observer.is_dirty(app.runtime()));
    assert_eq!(
        driver.publish_viewport(app.runtime_mut(), viewport(f64::NAN)),
        Err(SurfaceError::InvalidViewport)
    );
}

#[test]
fn repeated_urls_are_distinct_history_entries_and_new_navigation_discards_forward_entries() {
    let initial: Url = "app://chat/".parse().unwrap();
    let mut history = MemoryHistory::new(Some(initial.clone()));
    history.navigate("?chat=1", HistoryMode::Push).unwrap();
    history.navigate("?chat=1", HistoryMode::Push).unwrap();
    assert_eq!(history.go(-1).unwrap().query(), Some("chat=1"));
    assert_eq!(history.go(-1), Some(&initial));
    history.navigate("?chat=2", HistoryMode::Push).unwrap();
    assert!(history.go(1).is_none());
}

#[test]
fn passive_surfaces_reject_writes_and_request_admission_is_bounded() {
    let mut app = App::new(Empty);
    let passive = SurfaceDriver::install(
        &mut app,
        String::new(),
        None,
        SurfaceCapabilities::default(),
    );
    assert!(matches!(
        passive.surface().set_title(app.runtime_mut(), "no"),
        Err(SurfaceError::Unsupported)
    ));
    passive.close(app.runtime_mut());
    let active = SurfaceDriver::install(
        &mut app,
        String::new(),
        None,
        SurfaceCapabilities {
            title: true,
            navigation: true,
        },
    );
    let mut requests = Vec::new();
    for index in 0..128 {
        requests.push(
            active
                .surface()
                .set_title(app.runtime_mut(), index.to_string())
                .unwrap(),
        );
    }
    assert!(matches!(
        active.surface().set_title(app.runtime_mut(), "overflow"),
        Err(SurfaceError::Full)
    ));
    active.close(app.runtime_mut());
    assert!(requests
        .iter()
        .all(|request| request.status.get(app.runtime())
            == RequestStatus::Failed(SurfaceError::Closed)));
}

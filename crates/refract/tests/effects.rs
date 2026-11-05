#![cfg(feature = "macros")]

use refract::{prelude::*, App};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc};

type Log = Rc<RefCell<Vec<String>>>;

#[view]
fn subscription(cx: &mut ViewContext<'_>, period: u32, log: Log) -> impl View {
    #[effect(deps = period, captures(log))]
    let timer = |_| {
        log.borrow_mut().push(format!("start {period}"));
        let log = log.clone();
        Box::new(move || log.borrow_mut().push(format!("stop {period}")))
    };
    // The capture list clones the handle, leaving it available to the view.
    let _retained = log.clone();
    text(period.to_string())
}

#[test]
fn dependencies_restart_only_after_commit_and_cleanup_precedes_setup() {
    let log = Log::default();
    let mut app = App::new(subscription(1, log.clone()));
    let mut host = TestHost::default();
    drop(app.prepare().unwrap());
    assert!(log.borrow().is_empty());
    host.flush(&mut app).unwrap();
    assert_eq!(*log.borrow(), ["start 1"]);

    app.set_view(subscription(1, log.clone()));
    host.flush(&mut app).unwrap();
    assert_eq!(*log.borrow(), ["start 1"]);

    app.set_view(subscription(2, log.clone()));
    drop(app.prepare().unwrap());
    assert_eq!(
        *log.borrow(),
        ["start 1"],
        "discarding a candidate must not stop the live effect"
    );
    host.flush(&mut app).unwrap();
    assert_eq!(*log.borrow(), ["start 1", "stop 1", "start 2"]);

    app.set_view(Empty);
    host.flush(&mut app).unwrap();
    app.dispose();
    assert_eq!(*log.borrow(), ["start 1", "stop 1", "start 2", "stop 2"]);
}

// Also compile the macro's configurable facade path and exercise a setup that
// writes state using its inferred Runtime argument. No capture list is required
// when the setup may take ownership of the local handles.
#[view(crate_path = refract)]
fn initialize(cx: &mut ViewContext<'_>, value: u32, log: Log) -> impl View {
    #[state]
    let count: u32 = 0;
    let current = cx.read(&count);
    #[effect]
    let initialize = |rt| {
        count.set_if_changed(rt, value);
        log.borrow_mut().push(format!("start {value}"));
        let log = log.clone();
        Box::new(move || log.borrow_mut().push(format!("stop {value}")))
    };
    text(current.to_string())
}

#[test]
fn omitted_dependencies_run_once_per_mount_and_setup_can_write_state() {
    let log = Log::default();
    let mut app = App::new(initialize(7, log.clone()));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "7");
    app.set_view(initialize(9, log.clone()));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "7");
    assert_eq!(*log.borrow(), ["start 7"]);
    app.dispose();
    app.dispose();
    assert_eq!(*log.borrow(), ["start 7", "stop 7"]);
}

//! A clock driven by a mounted effect and an injected platform clock source.
use refract::{prelude::*, Cleanup, ScopeToken};
use std::rc::Rc;

const INK: Color = Color::rgb(38, 46, 35);
const MUTED: Color = Color::rgb(111, 116, 104);
const GREEN: Color = Color::rgb(53, 76, 44);
const LINE: Color = Color::rgb(217, 220, 207);
const PAPER: Color = Color::rgb(250, 250, 244);
const MONO: &str = "'SFMono-Regular', Consolas, monospace";

#[derive(Clone, PartialEq, Debug)]
pub struct Reading {
    pub time: String,
    pub date: String,
}

/// Shared through the environment; its binding keeps reads graph-tracked.
struct ClockModel {
    reading: Binding<Reading>,
}

#[view]
fn time_display(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let clock: Rc<ClockModel>;
    let reading = cx.read(&clock.reading);
    // Ordinary text, rather than a live region announcing every second.
    text(reading.time)
        .font_family(MONO)
        .font_size(42.0)
        .line_height(68.0)
        .foreground(INK)
        .text_align(Alignment::Center)
}

#[view]
fn date_display(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let clock: Rc<ClockModel>;
    let reading = cx.read(&clock.reading);
    text(reading.date)
        .font_size(14.0)
        .foreground(MUTED)
        .text_align(Alignment::Center)
}

// Layout components need neither clock props nor knowledge of the timer.
fn clock_panel() -> impl View {
    v_stack()
        .gap(24.0)
        .child(
            text("LOCAL TIME  /  24 HOUR")
                .font_family(MONO)
                .font_size(10.0)
                .foreground(MUTED)
                .text_align(Alignment::Center),
        )
        .child(time_display())
        .child(date_display())
        .padding(Insets::symmetric(24.0, 40.0))
        .background(PAPER)
        .border(Border::all(1.0, LINE))
        .radius(18.0)
}

/// The clock source is injected so the view can also be exercised by a native test host.
pub fn application(
    initial: Reading,
    subscribe: impl Fn(Binding<Reading>, ScopeToken) -> Cleanup + 'static,
) -> impl View {
    clock_root(initial, Rc::new(subscribe))
}

type Subscribe = dyn Fn(Binding<Reading>, ScopeToken) -> Cleanup;

#[view]
fn clock_root(cx: &mut ViewContext<'_>, initial: Reading, subscribe: Rc<Subscribe>) -> impl View {
    // The root owns state and a stable model, but does not read the value.
    // Only the descendant displays subscribe to changes in `reading`.
    #[state]
    let reading: Reading = initial;

    let model = cx.remember("clock-model", |_| ClockModel {
        reading: reading.clone(),
    });
    let scope = cx.scope();

    #[effect(id = "one-second-clock", captures(subscribe, reading, scope))]
    let clock_timer = |_| subscribe(reading.clone(), scope.clone());

    app_content().environment_object(model)
}

fn app_content() -> impl View {
    v_stack().gap(42.0)
        .child(h_stack().alignment(Alignment::Center).justify(Alignment::SpaceBetween)
            .child(h_stack().gap(10.0).alignment(Alignment::Center)
                .child(text("r").font_family("Georgia, serif").font_size(30.0).foreground(GREEN))
                .child(text("refract").font_size(18.0).font_weight(600).foreground(INK)))
            .child(text("EXAMPLE  /  002").font_family(MONO).font_size(10.0).foreground(MUTED)))
        .child(v_stack().gap(14.0)
            .child(text("A moment in time.").heading(1).font_family("Georgia, serif")
                .font_size(40.0).line_height(50.0).foreground(INK))
            .child(text("A living clock, one second at a time.\nYour local time, rendered with Refract.")
                .font_size(15.0).line_height(25.0).foreground(MUTED)))
        .child(clock_panel())
        .child(h_stack().justify(Alignment::SpaceBetween).alignment(Alignment::Center)
            .child(text("●  Updates every second").font_family(MONO).font_size(10.0).foreground(GREEN))
            .child(text(if cfg!(target_arch = "wasm32") { "Rust + WebAssembly" } else { "Rust + Native GPU" }).font_size(11.0).foreground(MUTED)))
        .max_width(480.0).fill_width().padding(28.0)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use refract_web::UpdateHandle;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::{prelude::*, JsCast};

    fn now() -> Reading {
        let date = js_sys::Date::new_0();
        let weekdays = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ];
        let months = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        Reading {
            time: format!(
                "{:02}:{:02}:{:02}",
                date.get_hours(),
                date.get_minutes(),
                date.get_seconds()
            ),
            date: format!(
                "{}, {} {}, {}",
                weekdays[date.get_day() as usize],
                months[date.get_month() as usize],
                date.get_date(),
                date.get_full_year()
            ),
        }
    }

    #[wasm_bindgen]
    pub struct ClockApp {
        mount: refract_web::Mount,
        running: Rc<Cell<bool>>,
        ticks: Rc<Cell<u32>>,
    }
    #[wasm_bindgen]
    impl ClockApp {
        pub fn dispose(&self) {
            self.mount.dispose();
        }
        pub fn is_running(&self) -> bool {
            self.running.get()
        }
        pub fn ticks(&self) -> u32 {
            self.ticks.get()
        }
        pub fn revision(&self) -> u64 {
            self.mount.revision()
        }
        pub fn dom_calls(&self) -> u64 {
            self.mount.dom_calls()
        }
    }

    #[wasm_bindgen]
    pub fn start(host: web_sys::Element) -> Result<ClockApp, JsValue> {
        let window = web_sys::window()
            .ok_or_else(|| JsValue::from_str("clock requires a browser window"))?;
        // The effect starts during the first commit. Its callback is connected
        // to a scoped update handle immediately after mount returns, before the
        // browser can dispatch the first interval task.
        let connection: Rc<RefCell<Option<UpdateHandle>>> = Rc::default();
        let owner = Rc::new(RefCell::new(None));
        let setup_error = Rc::new(RefCell::new(None));
        let running = Rc::new(Cell::new(false));
        let ticks = Rc::new(Cell::new(0_u32));
        let subscribe = {
            let connection = connection.clone();
            let owner = owner.clone();
            let setup_error = setup_error.clone();
            let running = running.clone();
            let ticks = ticks.clone();
            move |reading: Binding<Reading>, scope: ScopeToken| -> Cleanup {
                *owner.borrow_mut() = Some(scope);
                let connection = connection.clone();
                let ticks = ticks.clone();
                let callback = Closure::<dyn FnMut()>::new(move || {
                    let Some(handle) = connection.borrow().clone() else {
                        return;
                    };
                    let value = now(); // Re-read wall time: never count elapsed callbacks.
                    let reading = reading.clone();
                    if let Err(error) = handle.update(move |rt| reading.set_if_changed(rt, value)) {
                        web_sys::console::error_1(&error);
                    }
                    ticks.set(ticks.get().saturating_add(1));
                });
                match window.set_interval_with_callback_and_timeout_and_arguments_0(
                    callback.as_ref().unchecked_ref(),
                    1000,
                ) {
                    Ok(id) => {
                        running.set(true);
                        let window = window.clone();
                        let running = running.clone();
                        Box::new(move || {
                            window.clear_interval_with_handle(id);
                            running.set(false);
                            drop(callback);
                        })
                    }
                    Err(error) => {
                        *setup_error.borrow_mut() = Some(error);
                        Box::new(|| {})
                    }
                }
            }
        };
        let mount = refract_web::mount(
            host,
            application(now(), subscribe),
            refract_web::MountOptions::default(),
        )?;
        if let Some(error) = setup_error.borrow_mut().take() {
            mount.dispose();
            return Err(error);
        }
        let scope = owner
            .borrow_mut()
            .take()
            .ok_or_else(|| JsValue::from_str("clock effect was not mounted"))?;
        *connection.borrow_mut() = Some(mount.scoped_handle(scope));
        Ok(ClockApp {
            mount,
            running,
            ticks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract::{App, Patch};
    use refract_test::TestHost;
    use std::cell::{Cell, RefCell};

    fn reading(time: &str, date: &str) -> Reading {
        Reading {
            time: time.into(),
            date: date.into(),
        }
    }

    #[test]
    fn ticks_patch_only_clock_text_and_midnight_updates_the_date() {
        let binding = Rc::new(RefCell::new(None));
        let mut app = App::new(application(reading("23:59:58", "October 31"), {
            let binding = binding.clone();
            move |value, _| {
                *binding.borrow_mut() = Some(value);
                Box::new(|| {})
            }
        }));
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        let binding = binding.borrow().clone().unwrap();
        app.mutate(|rt| binding.set_if_changed(rt, reading("23:59:59", "October 31")));
        host.flush(&mut app).unwrap();
        assert!(
            matches!(host.updates.last().unwrap().patches.as_slice(), [Patch::Update(_, e)] if e.text == "23:59:59")
        );
        // Only the two context consumers and their text primitives evaluate;
        // the root, provider, and intermediate layout stay untouched.
        assert_eq!(app.stats().evaluated, 4);
        app.mutate(|rt| binding.set_if_changed(rt, reading("00:00:00", "November 1")));
        host.flush(&mut app).unwrap();
        assert_eq!(host.updates.last().unwrap().patches.len(), 2);
        assert!(host.text().contains("00:00:00 November 1"));
    }

    #[test]
    fn root_reevaluation_preserves_context_state_and_timer() {
        let starts = Rc::new(Cell::new(0));
        let binding = Rc::new(RefCell::new(None));
        let root = || {
            application(reading("12:00:00", "October 31"), {
                let starts = starts.clone();
                let binding = binding.clone();
                move |value, _| {
                    starts.set(starts.get() + 1);
                    *binding.borrow_mut() = Some(value);
                    Box::new(|| {})
                }
            })
        };
        let mut app = App::new(root());
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        let updates = binding.borrow().clone().unwrap();
        app.mutate(|rt| updates.set_if_changed(rt, reading("12:00:01", "October 31")));
        host.flush(&mut app).unwrap();

        app.set_view(root());
        host.flush(&mut app).unwrap();
        assert_eq!(
            starts.get(),
            1,
            "root evaluation must not restart the timer"
        );
        assert!(host.text().contains("12:00:01"), "state must not reset");

        // The original timer binding still reaches the context consumers.
        app.mutate(|rt| updates.set_if_changed(rt, reading("12:00:02", "October 31")));
        host.flush(&mut app).unwrap();
        assert!(host.text().contains("12:00:02"));
        assert_eq!(app.stats().evaluated, 4);
    }

    #[test]
    fn subscription_starts_after_commit_and_stops_once_on_unmount() {
        let starts = Rc::new(Cell::new(0));
        let stops = Rc::new(Cell::new(0));
        let scope = Rc::new(RefCell::new(None));
        let mut app = App::new(application(reading("12:00:00", "October 31"), {
            let starts = starts.clone();
            let stops = stops.clone();
            let scope = scope.clone();
            move |_, owner| {
                starts.set(starts.get() + 1);
                *scope.borrow_mut() = Some(owner);
                let stops = stops.clone();
                Box::new(move || stops.set(stops.get() + 1))
            }
        }));
        drop(app.prepare().unwrap());
        assert_eq!(starts.get(), 0);
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        assert_eq!(starts.get(), 1);
        assert!(scope.borrow().as_ref().unwrap().is_alive());
        app.set_view(Empty);
        host.flush(&mut app).unwrap();
        app.dispose();
        assert_eq!(stops.get(), 1);
        assert!(!scope.borrow().as_ref().unwrap().is_alive());
    }
}

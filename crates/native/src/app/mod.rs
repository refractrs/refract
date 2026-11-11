//! Winit lifecycle and UI-thread coordination. GPU drawing is delegated to Renderer.
mod events;

#[cfg(feature = "vello")]
use crate::renderer::VelloRenderer;
use crate::{
    host::Host,
    platform::TextInputSession,
    renderer::{Point, Presentation, Renderer},
    scheduler::Scheduler,
    NativeError, NativeHandle, Result,
};

use refract_core::{App, Color, ElementId, View};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowId},
};

/// Window configuration in logical pixels. The first adapter centers its root
/// content; the root can opt into filling the viewport with fill_width/fill_height.
#[derive(Clone, Debug)]
pub struct NativeOptions {
    pub title: String,
    /// Optional base for application navigation (for example app://chat/).
    pub initial_url: Option<refract_core::Url>,
    pub width: f64,
    pub height: f64,
    pub background: Color,
    /// Emit compact per-commit evaluation/patch/shaping counters to stderr.
    pub diagnostics: bool,
}

impl Default for NativeOptions {
    fn default() -> Self {
        Self {
            title: "Refract".into(),
            initial_url: None,
            width: 620.0,
            height: 760.0,
            background: Color::rgb(243, 244, 235),
            diagnostics: false,
        }
    }
}

/// Run a native app on the main thread using Vello's GPU renderer.
#[cfg(feature = "vello")]
pub fn run<V: View>(options: NativeOptions, view: impl FnOnce(NativeHandle) -> V) -> Result<()> {
    run_with_renderer(options, view, |window| {
        Ok(Box::new(VelloRenderer::new(window)?))
    })
}

/// Inject a renderer without changing the retained host, layout, or app code.
/// The factory runs after Winit creates a window. It can initialize its own wgpu
/// device and surface; the resulting renderer remains on the UI thread.
pub fn run_with_renderer<V: View>(
    options: NativeOptions,
    view: impl FnOnce(NativeHandle) -> V,
    factory: impl FnOnce(Arc<Window>) -> Result<Box<dyn Renderer>> + 'static,
) -> Result<()> {
    if !options.width.is_finite()
        || !options.height.is_finite()
        || options.width <= 0.0
        || options.height <= 0.0
    {
        return Err(NativeError::Host(
            "window dimensions must be finite and positive".into(),
        ));
    }
    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .map_err(host_error)?;
    let proxy = event_loop.create_proxy();
    let scheduler = Scheduler::new(move || {
        let _ = proxy.send_event(());
    });
    let mut app = App::new(view(scheduler.handle()));
    let surface = Rc::new(crate::surface::NativeSurface::new(
        &mut app,
        options.title.clone(),
        options.initial_url.clone(),
    ));
    scheduler.set_surface(surface.clone());
    let proxy = event_loop.create_proxy();
    app.runtime_mut().set_waker(move || {
        let _ = proxy.send_event(());
    });
    let message_proxy = event_loop.create_proxy();
    app.runtime_mut().set_message_waker(move || {
        let _ = message_proxy.send_event(());
    });
    let image_proxy = event_loop.create_proxy();
    let host = Host::with_waker(move || {
        let _ = image_proxy.send_event(());
    })?;
    let mut runner = NativeApp {
        options,
        app,
        surface,
        scheduler: Some(scheduler),
        host,
        window: None,
        renderer: None,
        rendered_size: None,
        cursor_inside: false,
        factory: Some(Box::new(factory)),
        cursor: Point::default(),
        pressed: None,
        modifiers: ModifiersState::empty(),
        pointer_buttons: refract_core::PointerButtons::default(),
        touch_presses: std::collections::HashMap::new(),
        clipboard: None,
        text_input: TextInputSession::default(),
        window_focused: true,
        needs_frame: true,
        occluded: false,
        retry_at: None,
        error: None,
        presented: 0,
    };
    event_loop.run_app(&mut runner).map_err(host_error)?;
    runner.surface.driver.close(runner.app.runtime_mut());
    runner.app.dispose();
    runner.scheduler = None;
    runner.error.map_or(Ok(()), Err)
}

type Factory = Box<dyn FnOnce(Arc<Window>) -> Result<Box<dyn Renderer>>>;

struct NativeApp {
    options: NativeOptions,
    app: App,
    surface: Rc<crate::surface::NativeSurface>,
    scheduler: Option<Scheduler>,
    host: Host,
    // Renderer drops before the final window owner.
    renderer: Option<Box<dyn Renderer>>,
    rendered_size: Option<winit::dpi::PhysicalSize<u32>>,
    cursor_inside: bool,
    window: Option<Arc<Window>>,
    factory: Option<Factory>,
    cursor: Point,
    pressed: Option<ElementId>,
    modifiers: ModifiersState,
    pointer_buttons: refract_core::PointerButtons,
    touch_presses: std::collections::HashMap<u64, (Option<ElementId>, Point)>,
    clipboard: Option<arboard::Clipboard>,
    text_input: TextInputSession,
    window_focused: bool,
    needs_frame: bool,
    occluded: bool,
    retry_at: Option<Instant>,
    error: Option<NativeError>,
    presented: u64,
}

impl NativeApp {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: NativeError) {
        // A failed host apply is not an accepted diff baseline. Stop rather than
        // trying to continue from a potentially partially mutated retained tree.
        self.error = Some(error);
        self.surface.driver.close(self.app.runtime_mut());
        self.app.dispose();
        self.scheduler = None;
        event_loop.exit();
    }

    fn flush(&mut self) -> Result<()> {
        if let Some(window) = &self.window {
            self.surface.sample(self.app.runtime_mut(), window);
        }
        for (id, event) in self.host.take_input_events() {
            self.app.dispatch(id, event);
        }
        self.needs_frame |= self.host.poll_images()?;
        if let Some(scheduler) = &self.scheduler {
            self.app.mutate(|rt| scheduler.drain(rt, Instant::now()));
        }
        self.app.runtime_mut().drain_messages();
        if let Some(pending) = self.app.prepare()? {
            let stats = pending.update.stats;
            self.needs_frame |= self.host.apply(&pending.update)?;
            // Commit even an empty patch batch: callbacks and effects can change
            // without a visible property change. Presentation is a later phase.
            self.app.commit(pending)?;
            if self.options.diagnostics {
                eprintln!("refract native commit={} evaluated={} visited={} patches={} created={} removed={}",
                    self.app.revision(), stats.evaluated, stats.visited, stats.patches, stats.created, stats.removed);
            }
        }
        if let Some(window) = &self.window {
            self.surface.apply(self.app.runtime_mut(), window);
        }
        Ok(())
    }

    fn activate(&mut self, id: ElementId) {
        if let Some(event) = self.host.activation(id) {
            self.app.dispatch(id, event);
        }
    }

    fn draw(&mut self) -> Result<()> {
        let (Some(window), Some(renderer)) = (&self.window, &mut self.renderer) else {
            return Ok(());
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 || self.occluded {
            return Ok(());
        }
        #[cfg(feature = "debug-damage")]
        if !self.needs_frame
            && self.rendered_size == Some(size)
            && renderer
                .redraw_deadline()
                .is_some_and(|deadline| deadline <= Instant::now())
        {
            if let Some(presentation) = renderer.refresh()? {
                match presentation {
                    Presentation::Presented => {
                        self.presented += 1;
                        self.retry_at = None;
                    }
                    Presentation::Retry => {
                        self.needs_frame = true;
                        self.retry_at = Some(Instant::now() + Duration::from_millis(100));
                    }
                }
                return Ok(());
            }
        }
        // Use one latest-size snapshot for both surface allocation and layout.
        // Resize events can queue faster than GPU frames during a live drag.
        if self.rendered_size != Some(size) {
            renderer.resize(size.width, size.height)?;
            self.rendered_size = Some(size);
        }
        let scale = window.scale_factor();
        let frame = self.host.frame(
            (size.width as f64 / scale) as f32,
            (size.height as f64 / scale) as f32,
            scale,
            self.options.background,
        )?;
        self.text_input.sync(
            window,
            self.host.focus.filter(|id| {
                self.window_focused && !self.host.menu_is_open() && self.host.is_editor(*id)
            }),
            self.host.ime_area(),
        );
        self.host.refresh_mouse_boundary(
            self.cursor,
            self.cursor_inside && self.window_focused && !self.host.menu_is_open(),
        );
        // Layout/scrolling may move a button beneath a stationary pointer.
        let hover = (self.cursor_inside
            && !self.host.menu_is_open()
            && !self.host.scrollbar_hit(self.cursor))
        .then(|| self.host.hit_test(self.cursor))
        .flatten();
        let hover_changed = self.host.set_hover(hover);
        // Resize can move an edge away from a stationary pointer. Reconcile
        // scrollbar hover against the new geometry so it can still fade out.
        let scrollbar_hover_changed = self.host.reconcile_scrollbar_hover(
            (self.cursor_inside && !self.host.menu_is_open()).then_some(self.cursor),
        );
        match renderer.render(&frame)? {
            Presentation::Presented => {
                self.presented += 1;
                self.needs_frame = hover_changed || scrollbar_hover_changed;
                self.retry_at = None;
                if self.options.diagnostics {
                    let stats = self.host.stats();
                    eprintln!(
                        "refract native frame={} elements={} shapes={} layouts={} images={}",
                        self.presented,
                        stats.elements,
                        stats.text_shapes,
                        stats.layout_passes,
                        stats.images_decoded
                    );
                }
            }
            Presentation::Retry => {
                self.needs_frame = true;
                self.retry_at = Some(Instant::now() + Duration::from_millis(100));
            }
        }
        // Dispatch only after the frame has finished borrowing host/renderer.
        // App dispatch itself does not dirty views; only callback state writes do.
        for (id, event) in self.host.take_scroll_events() {
            self.app.dispatch(id, event);
        }

        Ok(())
    }
}

impl ApplicationHandler<()> for NativeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            self.occluded = false;
            self.needs_frame = true;
            return;
        }
        let result = (|| {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title(&self.options.title)
                            .with_inner_size(LogicalSize::new(
                                self.options.width,
                                self.options.height,
                            )),
                    )
                    .map_err(host_error)?,
            );
            self.renderer = Some(self
                .factory
                .take()
                .expect("renderer factory consumed once")(
                window.clone()
            )?);
            self.window = Some(window);
            self.flush()
        })();
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.occluded = true;
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _: ()) {
        // Wakes are coalesced by the single flush in about_to_wait.
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != id {
            return;
        }
        if self.input_event(&event, window.scale_factor()) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.surface.driver.close(self.app.runtime_mut());
                self.app.dispose();
                self.scheduler = None;
                event_loop.exit();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                // Request now, but keep GPU reallocations coalesced in draw().
                // On macOS Winit drains queued redraws BEFORE AboutToWait;
                // requesting only there adds another run-loop turn of latency.
                self.needs_frame = true;
                self.retry_at = None;
                self.host.close_menu();
                window.request_redraw();
            }
            WindowEvent::Occluded(value) => {
                self.occluded = value;
                self.needs_frame = true;
                if !value {
                    self.retry_at = None;
                    window.request_redraw();
                }
            }
            WindowEvent::Moved(_) => {
                // A move can cross displays or enter a snap preview without
                // changing the drawable size. Refresh native presentation
                // policy even when no Resized/ScaleFactorChanged event arrives.
                self.needs_frame = true;
                self.retry_at = None;
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.flush().and_then(|_| self.draw()) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.to_logical::<f32>(window.scale_factor());
                self.cursor = Point { x: p.x, y: p.y };
                self.cursor_inside = true;
                self.needs_frame |= self
                    .host
                    .scrollbar_pointer((!self.host.menu_is_open()).then_some(self.cursor));
                if self.host.menu_is_open() {
                    self.needs_frame |= self.host.menu_pointer(self.cursor);
                    window.set_cursor(winit::window::CursorIcon::Default);
                    return;
                }
                if self.host.scrollbar_drag(self.cursor) {
                    self.needs_frame = true;
                    return;
                }
                if self.host.scrollbar_hit(self.cursor) {
                    self.needs_frame |= self.host.set_hover(None);
                    window.set_cursor(winit::window::CursorIcon::Default);
                    return;
                }
                self.needs_frame |= self.host.set_hover(self.host.hit_test(self.cursor));
                if self.pressed.is_some_and(|id| self.host.is_editor(id)) {
                    self.host.editor_pointer(self.cursor, true);
                    self.needs_frame = true;
                }
                window.set_cursor(
                    if self
                        .host
                        .hit_test(self.cursor)
                        .is_some_and(|id| self.host.is_editor(id))
                    {
                        winit::window::CursorIcon::Text
                    } else if self.host.hit_test(self.cursor).is_some() {
                        winit::window::CursorIcon::Pointer
                    } else {
                        winit::window::CursorIcon::Default
                    },
                );
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_inside = false;
                self.needs_frame |= self.host.scrollbar_pointer(None);
                self.needs_frame |= self.host.set_hover(None);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed && self.host.menu_is_open() {
                    if let Some((id, event)) =
                        self.host.menu_click(self.cursor, &mut self.clipboard)
                    {
                        self.app.dispatch(id, event);
                    }
                    self.needs_frame = true;
                    return;
                }
                if state == ElementState::Pressed
                    && cfg!(target_os = "macos")
                    && self.modifiers.control_key()
                {
                    self.pressed = None;
                    self.host.open_edit_menu(self.cursor, false);
                    self.needs_frame = true;
                    return;
                }
                if state == ElementState::Pressed && self.host.scrollbar_press(self.cursor) {
                    self.pressed = None;
                    self.host.set_pressed(None);
                    self.needs_frame = true;
                    return;
                }
                if state == ElementState::Released && self.host.scrollbar_release() {
                    self.needs_frame = true;
                    return;
                }
                let target = self
                    .cursor_inside
                    .then(|| self.host.hit_test(self.cursor))
                    .flatten();
                if state == ElementState::Pressed {
                    self.pressed = target;
                    self.host.set_pressed(target);
                    self.host.focus_pointer(target);
                    self.host
                        .editor_pointer(self.cursor, self.modifiers.shift_key());
                    self.needs_frame = true;
                } else {
                    self.needs_frame |= self.host.set_pressed(None);
                    if let Some(id) = self.pressed.take().filter(|id| Some(*id) == target) {
                        self.activate(id);
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.pressed = None;
                self.host.close_menu();
                self.host.open_edit_menu(self.cursor, false);
                self.needs_frame = true;
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                if self.host.menu_is_open() {
                    return;
                }
                self.needs_frame |= self.host.reveal_scrollbars_at(self.cursor);
                let precise = matches!(delta, MouseScrollDelta::PixelDelta(_));
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Point {
                        x: x * 32.0,
                        y: y * 32.0,
                    },
                    MouseScrollDelta::PixelDelta(p) => {
                        let p = p.to_logical::<f32>(window.scale_factor());
                        Point { x: p.x, y: p.y }
                    }
                };
                self.needs_frame |= self.host.scroll_event(self.cursor, delta, phase, precise);
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(true) => {
                self.window_focused = true;
                self.needs_frame = true;
            }
            WindowEvent::Focused(false) => {
                self.window_focused = false;
                self.needs_frame = true;
                self.host.cancel_scroll();
                self.host.close_menu();
                self.pressed = None;
                self.cursor_inside = false;
                self.needs_frame |= self.host.set_pressed(None);
                self.needs_frame |= self.host.set_hover(None);
                self.host.editor_ime(&winit::event::Ime::Disabled);
            }
            WindowEvent::Ime(event) => {
                if !self.window_focused || self.host.menu_is_open() {
                    return;
                }
                if let Some(id) = self.host.focus {
                    if let Some(event) = self.host.editor_ime(&event) {
                        self.app.dispatch(id, event);
                    }
                    self.needs_frame = true;
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.host.menu_is_open() {
                    if let Some((id, message)) =
                        self.host
                            .menu_key(&event.logical_key, self.modifiers, &mut self.clipboard)
                    {
                        self.app.dispatch(id, message);
                    }
                    self.needs_frame = true;
                    return;
                }
                if event.logical_key == Key::Named(NamedKey::ContextMenu)
                    || (event.logical_key == Key::Named(NamedKey::F10)
                        && self.modifiers.shift_key())
                {
                    if let Some(area) = self.host.ime_area() {
                        self.needs_frame |= self.host.open_edit_menu(
                            Point {
                                x: area.x,
                                y: area.y + area.height,
                            },
                            true,
                        );
                    }
                    return;
                }
                if event.logical_key != Key::Named(NamedKey::Tab)
                    && self.host.focus.is_some_and(|id| self.host.is_editor(id))
                {
                    let id = self.host.focus.unwrap();
                    if let Some(message) =
                        self.host
                            .editor_key(&event, self.modifiers, &mut self.clipboard)
                    {
                        if !event.repeat || !matches!(message, refract_core::Event::Submit) {
                            self.app.dispatch(id, message);
                        }
                    }
                    self.needs_frame = true;
                    return;
                }
                if event.repeat {
                    return;
                }
                match event.logical_key {
                    Key::Named(NamedKey::Tab) => {
                        self.host.focus_next(self.modifiers.shift_key());
                        self.needs_frame = true;
                    }
                    Key::Named(NamedKey::Enter | NamedKey::Space) => {
                        if let Some(id) = self.host.focus {
                            self.activate(id);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.scheduler.is_none() {
            return;
        }
        if let Err(error) = self.flush() {
            self.fail(event_loop, error);
            return;
        }
        let now = Instant::now();
        let scrollbar = self.host.scrollbar_deadline().filter(|_| !self.occluded);
        #[cfg(feature = "debug-damage")]
        let renderer = self
            .renderer
            .as_ref()
            .and_then(|r| r.redraw_deadline())
            .filter(|_| !self.occluded);
        self.needs_frame |= scrollbar.is_some_and(|deadline| deadline <= now);
        if let Some(window) = &self.window {
            self.text_input.sync(
                window,
                self.host.focus.filter(|id| {
                    self.window_focused && !self.host.menu_is_open() && self.host.is_editor(*id)
                }),
                self.host.ime_area(),
            );
        }
        if let Some(window) = &self.window {
            let size = window.inner_size();
            let needs_frame = self.needs_frame;
            #[cfg(feature = "debug-damage")]
            let needs_frame = needs_frame || renderer.is_some_and(|deadline| deadline <= now);
            if needs_frame
                && !self.occluded
                && size.width > 0
                && size.height > 0
                && self.retry_at.is_none_or(|deadline| deadline <= now)
            {
                window.request_redraw();
            }
        }
        let deadline = self.scheduler.as_ref().and_then(Scheduler::deadline);
        let retry = self.retry_at.filter(|_| !self.occluded);
        let deadline = deadline
            .into_iter()
            .chain(retry)
            .chain(scrollbar.filter(|deadline| *deadline > now))
            .min();
        #[cfg(feature = "debug-damage")]
        let deadline = deadline
            .into_iter()
            .chain(renderer.filter(|deadline| *deadline > now))
            .min();
        event_loop.set_control_flow(match deadline {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        });
    }
}

fn host_error(error: impl std::fmt::Display) -> NativeError {
    NativeError::Host(error.to_string())
}

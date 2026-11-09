//! Window metadata and application history are independent of layout and painting.
use refract_core::{
    App, MemoryHistory, Runtime, SurfaceCapabilities, SurfaceCommand, SurfaceDriver, SurfaceError,
    Url, Viewport,
};
use std::cell::RefCell;
use winit::window::Window;

pub(crate) struct NativeSurface {
    pub driver: SurfaceDriver,
    history: RefCell<MemoryHistory>,
}

impl NativeSurface {
    pub fn new(app: &mut App, title: String, location: Option<Url>) -> Self {
        Self {
            driver: SurfaceDriver::install(
                app,
                title,
                location.clone(),
                SurfaceCapabilities {
                    title: true,
                    navigation: true,
                },
            ),
            history: RefCell::new(MemoryHistory::new(location)),
        }
    }

    pub fn sample(&self, rt: &mut Runtime, window: &Window) {
        let size = window.inner_size();
        let scale = window.scale_factor();
        // Winit supplies finite dimensions and a positive scale factor.
        let _ = self.driver.publish_viewport(
            rt,
            Viewport {
                width: f64::from(size.width) / scale,
                height: Some(f64::from(size.height) / scale),
                scale_factor: scale,
            },
        );
    }

    pub fn apply(&self, rt: &mut Runtime, window: &Window) {
        for request in self.driver.take_requests(rt) {
            let result = match &request.command {
                SurfaceCommand::SetTitle(title) => {
                    if self.driver.surface().title.get(rt) != *title {
                        window.set_title(title);
                    }
                    self.driver.publish_title(rt, title.clone());
                    Ok(())
                }
                SurfaceCommand::Navigate { reference, mode } => self
                    .history
                    .borrow_mut()
                    .navigate(reference, *mode)
                    .map(|url| {
                        self.driver.publish_location(rt, Some(url.clone()));
                    }),
            };
            request.complete(rt, result);
        }
    }

    pub fn traverse(&self, rt: &mut Runtime, delta: isize) {
        if let Some(url) = self.history.borrow_mut().go(delta) {
            self.driver.publish_location(rt, Some(url.clone()));
        }
    }

    pub fn feed_location(
        &self,
        rt: &mut Runtime,
        url: Url,
        mode: refract_core::HistoryMode,
    ) -> Result<(), SurfaceError> {
        let url = self
            .history
            .borrow_mut()
            .navigate(url.as_str(), mode)?
            .clone();
        self.driver.publish_location(rt, Some(url));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::{Empty, HistoryMode};

    #[test]
    fn incoming_locations_and_history_traversal_do_not_enqueue_navigation() {
        let mut app = App::new(Empty);
        let surface = NativeSurface::new(
            &mut app,
            "Test".into(),
            Some("app://test/".parse().unwrap()),
        );
        surface
            .feed_location(
                app.runtime_mut(),
                "app://test/second".parse().unwrap(),
                HistoryMode::Push,
            )
            .unwrap();
        assert_eq!(
            surface
                .driver
                .surface()
                .location
                .get(app.runtime())
                .unwrap()
                .path(),
            "/second"
        );
        surface.traverse(app.runtime_mut(), -1);
        assert_eq!(
            surface
                .driver
                .surface()
                .location
                .get(app.runtime())
                .unwrap()
                .path(),
            "/"
        );
        surface.traverse(app.runtime_mut(), 1);
        assert_eq!(
            surface
                .driver
                .surface()
                .location
                .get(app.runtime())
                .unwrap()
                .path(),
            "/second"
        );
        assert!(surface.driver.take_requests(app.runtime_mut()).is_empty());
    }
}

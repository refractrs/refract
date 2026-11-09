//! Deterministic in-memory adapter for Refract applications.
use refract_core::*;
use std::collections::HashMap;
#[derive(Default)]
pub struct TestHost {
    pub elements: HashMap<ElementId, Element>,
    pub children: HashMap<Option<ElementId>, Vec<ElementId>>,
    pub revision: u64,
    pub updates: Vec<Update>,
    pub surface: Option<SurfaceDriver>,
    pub history: MemoryHistory,
}
impl TestHost {
    pub fn apply(&mut self, update: &Update) -> Result<(), ViewError> {
        if update.base_revision != self.revision {
            return Err(ViewError::new("host revision mismatch"));
        }
        let mut elements = self.elements.clone();
        let mut children = self.children.clone();
        for patch in &update.patches {
            match patch {
                Patch::Create(id, element) => {
                    if elements.insert(*id, element.clone()).is_some() {
                        return Err(ViewError::new("duplicate element"));
                    }
                }
                Patch::Update(id, element) => {
                    *elements
                        .get_mut(id)
                        .ok_or_else(|| ViewError::new("unknown update target"))? = element.clone();
                }
                Patch::Children(parent, list) => {
                    if parent.is_some_and(|id| !elements.contains_key(&id))
                        || list.iter().any(|id| !elements.contains_key(id))
                    {
                        return Err(ViewError::new("unknown child/parent"));
                    }
                    children.insert(*parent, list.clone());
                }
                Patch::Remove(id) => {
                    elements.remove(id);
                    children.remove(&Some(*id));
                    for list in children.values_mut() {
                        list.retain(|child| child != id);
                    }
                }
            }
        }
        self.elements = elements;
        self.children = children;
        self.revision = update.revision;
        self.updates.push(update.clone());
        Ok(())
    }
    /// Opt into deterministic host metadata before the first render.
    pub fn install_surface(
        &mut self,
        app: &mut App,
        title: &str,
        location: Option<Url>,
    ) -> std::rc::Rc<Surface> {
        assert_eq!(self.revision, 0, "install the surface before rendering");
        if let Some(previous) = self.surface.take() {
            previous.close(app.runtime_mut());
        }
        let driver = SurfaceDriver::install(
            app,
            title.into(),
            location.clone(),
            SurfaceCapabilities {
                title: true,
                navigation: true,
            },
        );
        let surface = driver.surface();
        self.history = MemoryHistory::new(location);
        self.surface = Some(driver);
        surface
    }

    pub fn set_viewport(&self, app: &mut App, viewport: Viewport) -> Result<(), SurfaceError> {
        self.surface
            .as_ref()
            .ok_or(SurfaceError::Unsupported)?
            .publish_viewport(app.runtime_mut(), viewport)
    }

    pub fn go(&mut self, app: &mut App, delta: isize) {
        if let (Some(driver), Some(url)) = (&self.surface, self.history.go(delta)) {
            driver.publish_location(app.runtime_mut(), Some(url.clone()));
        }
    }

    /// Drain one message batch, then settle committed surface effects and their observations.
    /// Returns whether any view update committed; host-only requests may also have executed.
    pub fn flush(&mut self, app: &mut App) -> Result<bool, ViewError> {
        app.runtime_mut().drain_messages();
        let mut changed = false;
        for _ in 0..32 {
            let pending = app.prepare()?;
            let committed = pending.is_some();
            if let Some(pending) = pending {
                self.apply(&pending.update)?;
                app.commit(pending)?;
                changed = true;
            }
            let mut applied = false;
            if let Some(driver) = &self.surface {
                for request in driver.take_requests(app.runtime_mut()) {
                    applied = true;
                    let result = match &request.command {
                        SurfaceCommand::SetTitle(title) => {
                            driver.publish_title(app.runtime_mut(), title.clone());
                            Ok(())
                        }
                        SurfaceCommand::Navigate { reference, mode } => {
                            self.history.navigate(reference, *mode).map(|url| {
                                driver.publish_location(app.runtime_mut(), Some(url.clone()));
                            })
                        }
                    };
                    request.complete(app.runtime_mut(), result);
                }
            }
            if self.surface.is_none() || (!committed && !applied) {
                return Ok(changed);
            }
        }
        Err(ViewError::new("surface effects did not settle"))
    }
    pub fn text(&self) -> String {
        fn walk(host: &TestHost, parent: Option<ElementId>, out: &mut Vec<String>) {
            for id in host.children.get(&parent).into_iter().flatten() {
                let e = &host.elements[id];
                if !e.text.is_empty() {
                    out.push(e.text.clone());
                }
                walk(host, Some(*id), out)
            }
        }
        let mut out = vec![];
        walk(self, None, &mut out);
        out.join(" ")
    }
    pub fn button(&self, label: &str) -> ElementId {
        *self
            .elements
            .iter()
            .find(|(_, e)| {
                e.interaction == Some(Interaction::Activate)
                    && e.role.as_deref() == Some("button")
                    && e.label.as_deref() == Some(label)
            })
            .expect("button not found")
            .0
    }
    pub fn press(&mut self, app: &mut App, label: &str) -> Result<(), ViewError> {
        app.dispatch(self.button(label), Event::Press);
        self.flush(app)?;
        Ok(())
    }
}

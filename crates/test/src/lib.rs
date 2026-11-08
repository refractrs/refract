//! Deterministic in-memory adapter for Refract applications.
use refract_core::*;
use std::collections::HashMap;
#[derive(Default)]
pub struct TestHost {
    pub elements: HashMap<ElementId, Element>,
    pub children: HashMap<Option<ElementId>, Vec<ElementId>>,
    pub revision: u64,
    pub updates: Vec<Update>,
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
    /// Drain one bounded message batch and commit affected views. Returns whether
    /// a view update was committed, not whether messages were delivered. Use
    /// `app.runtime().has_pending_messages()` to drive additional drains explicitly.
    pub fn flush(&mut self, app: &mut App) -> Result<bool, ViewError> {
        app.runtime_mut().drain_messages();
        let Some(pending) = app.prepare()? else {
            return Ok(false);
        };
        self.apply(&pending.update)?;
        app.commit(pending)?;
        Ok(true)
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
